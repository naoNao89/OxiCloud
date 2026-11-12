#!/usr/bin/env bash
# =============================================================================
# burst-trials.sh <base-port> <outdir> <label> [n] [trials]
#   The atomicity question (H8) at the binary level, repeated: how many of N
#   SIMULTANEOUS requests does a `max_requests = 1` limiter serve?
#
#   One trial = boot a fresh instance (empty in-memory limiter) with
#   OXICLOUD_RATE_LIMIT_REGISTER_MAX=1 / WINDOW_SECS=15, release N curl
#   processes at a common barrier, then count 400 (served) vs 429 (rejected) on
#   the setup route. The app is restarted per trial because the limiter is an
#   in-process moka cache and has no reset endpoint — without the restart a
#   second burst in the same window would measure an already-exhausted bucket.
#
#   This is a case list on top of docs/re/scripts/lib-re.sh (the same boot, DB,
#   redaction and log plumbing as run-corpus.sh), not a second harness.
#
#   Output: <outdir>/<label>-burst.tsv   one line per trial:
#       trial, n, served_400, rejected_429, other, multiset, df
#   and <outdir>/<label>-burst.json  the same as JSON, plus per-trial app-log
#   evidence that the limiter (not something else) produced the 429s.
#
#   READ IT AS: served_400 > 1 is an observed overshoot for that trial. A run of
#   trials with served_400 == 1 does NOT prove atomicity — it is the absence of
#   an observed violation. Served counts are the invariant (`<= max`), not an
#   interleaving claim.
# =============================================================================
set -uo pipefail
HERE=$(cd "$(dirname "$0")" && pwd)
# shellcheck source=lib-re.sh
. "$HERE/lib-re.sh"

PORT=${1:?usage: burst-trials.sh <port> <outdir> <label> [n] [trials]}
OUT=${2:?usage: burst-trials.sh <port> <outdir> <label> [n] [trials]}
LABEL=${3:?usage: burst-trials.sh <port> <outdir> <label> [n] [trials]}
N=${4:-64}
T=${5:-5}

RE_HOST=127.0.0.1
RE_PORT=$PORT
export RE_HOST RE_PORT
re_template_ensure || exit 1
mkdir -p "$OUT"
TSV="$OUT/$LABEL-burst.tsv"
JSON="$OUT/$LABEL-burst.json"

FX=$(mktemp -d); trap 'rm -rf "$FX"' EXIT
printf '%s' '{"username":"admin","email":"re-test@example.test","password":"1234567"}' > "$FX/pw7.json"
printf 'trial\tn\tserved_400\trejected_429\tother\tmultiset\tsecs\n' > "$TSV"

echo "# burst-trials label=$LABEL bin=$RE_BIN n=$N trials=$T" | tee "$OUT/$LABEL-burst.run.txt"
echo "# bin sha256 $(sha256sum "$RE_BIN" | cut -d' ' -f1)" | tee -a "$OUT/$LABEL-burst.run.txt"
for t in $(seq 1 "$T"); do
  db="re_burst_${LABEL}_$t"
  boot="${LABEL}-t$t"
  re_db_reset "$db" >/dev/null 2>&1 || { echo "db reset failed"; exit 1; }
  # Both budgets are pinned to max=1: patch 0002 gives /api/setup its OWN limiter
  # (OXICLOUD_RATE_LIMIT_SETUP_*), so on a tree that carries it
  # OXICLOUD_RATE_LIMIT_REGISTER_MAX alone no longer constrains the setup route
  # (measured: with only the register var set, the patched build lets its default
  # 5 setup requests through a 64-way burst). On v0.10.0/HEAD-without-0002 the
  # SETUP_* names do not exist and are silently ignored, so both builds run the
  # same max=1 ceiling.
  re_boot "$boot" "$PORT" "$db" \
    "OXICLOUD_RATE_LIMIT_REGISTER_MAX=1" "OXICLOUD_RATE_LIMIT_REGISTER_WINDOW_SECS=15" \
    "OXICLOUD_RATE_LIMIT_SETUP_MAX=1" "OXICLOUD_RATE_LIMIT_SETUP_WINDOW_SECS=15" >/dev/null ||
    { echo "boot $boot failed"; exit 1; }
  sleep 0.5
  # barrier: every curl waits for one file, then they all go at once
  barrier=$(mktemp); rm -f "$barrier"
  results=$(mktemp -d)
  start=$(date +%s.%N)
  for i in $(seq 1 "$N"); do
    ( while [ ! -f "$barrier" ]; do sleep 0.02; done
      "$RE_CURL" -s -o /dev/null -w '%{http_code}\n' -m 20 \
        -H 'Content-Type: application/json' --data-binary @"$FX/pw7.json" \
        "http://$RE_HOST:$RE_PORT/api/setup" > "$results/$i" ) &
  done
  sleep 0.3; touch "$barrier"; wait
  end=$(date +%s.%N)
  served=$(cat "$results"/* 2>/dev/null | grep -c '^400$')
  rejected=$(cat "$results"/* 2>/dev/null | grep -c '^429$')
  other=$(ls "$results" | wc -l)
  other=$(( other - served - rejected ))
  multiset=$(cat "$results"/* | sort -n | uniq -c | awk '{printf "%sx%s ", $1, $2}')
  secs=$(awk -v a="$start" -v b="$end" 'BEGIN{printf "%.3f", b-a}')
  printf '%d\t%d\t%d\t%d\t%d\t%s\t%s\n' "$t" "$N" "$served" "$rejected" "$other" "$multiset" "$secs" >> "$TSV"
  rechk=$(grep -c 'Rate limit exceeded' "$RE_ROOT/run-$boot/app.log" 2>/dev/null || echo 0)
  echo "  trial $t: served=$served rejected=$rejected other=$other [$multiset] in ${secs}s; app-log 'Rate limit exceeded' lines=$rechk"
  re_shutdown "$boot" >/dev/null 2>&1
  rm -rf "$results"
done

python3 - "$TSV" "$JSON" "$LABEL" "$N" "$RE_BIN" <<'PY'
import json, sys, hashlib, os
tsv, out, label, n, binp = sys.argv[1:6]
rows = [l.rstrip('\n').split('\t') for l in open(tsv) if l.strip() and not l.startswith('trial')]
doc = {'label': label, 'n': int(n), 'binary': binp,
       'binary_sha256': hashlib.sha256(open(binp, 'rb').read()).hexdigest(),
       'posture': {'OXICLOUD_RATE_LIMIT_REGISTER_MAX': '1',
                   'OXICLOUD_RATE_LIMIT_REGISTER_WINDOW_SECS': '15'},
       'trials': [{'trial': int(r[0]), 'n': int(r[1]), 'served_400': int(r[2]),
                   'rejected_429': int(r[3]), 'other': int(r[4]),
                   'multiset': r[5].strip(), 'secs': float(r[6])} for r in rows]}
served = [t['served_400'] for t in doc['trials']]
doc['overshoot_trials'] = sum(1 for s in served if s > 1)
doc['trials_total'] = len(served)
doc['max_served'] = max(served) if served else None
json.dump(doc, open(out, 'w'), indent=1)
print('wrote %s: overshoot in %d/%d trials, max served %s (invariant: <= 1)'
      % (out, doc['overshoot_trials'], doc['trials_total'], doc['max_served']))
PY

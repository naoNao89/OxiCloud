#!/usr/bin/env bash
# =============================================================================
# run-rate-limit-experiments.sh — timing / budget / concurrency experiments for
# the OxiCloud 0.10.0 RELEASE-POSTURE differential.
#
#   usage: run-rate-limit-experiments.sh <base_url> <outdir> <label>
#          e.g. run-rate-limit-experiments.sh http://127.0.0.1:18086 \
#                   /data/re-scratch/out/A A
#
# WHAT IT DOES
#   Boots the binary under test (RE_BIN) several times, once per POSTURE, and
#   writes ONE JSONL PER POSTURE so a number can never be attributed to the
#   wrong config:
#
#     <outdir>/<label>-rl3600.jsonl     release posture, register 5/3600 s
#                                       -> case 20 retry-after-constant
#     <outdir>/<label>-regwin15.jsonl   REGISTER_WINDOW_SECS=3600, MAX=100
#                                       -> case 18b unconfounded concurrency
#                                       -> compression bracket (raw vs
#                                          transferred 400 bodies under
#                                          Accept-Encoding: gzip)
#     <outdir>/<label>-max1win15.jsonl  REGISTER_MAX=1, WINDOW_SECS=15
#                                       -> case 19 ttl-refresh (+ its control)
#                                       -> case 18 concurrent-burst-rejects
#     <outdir>/<label>-xff.jsonl        TRUST_PROXY_CIDR=127.0.0.1/32,
#                                       release posture
#                                       -> case 15 xff-honoured-trusted
#
#   Each record's `config` object repeats the exact rate-limit / trust-proxy
#   variables of the instance that produced it, read back from that instance's
#   recorded env file — so "which config produced this number" is answerable
#   from the JSONL alone.
#
# PORTS: <base_url> port +1..+4 (18087..18090 by default). Every instance gets
# its own port, DB, storage dir and app log; re_boot refuses to start if the
# port is already answered by a foreign process.
#
# TIME BUDGET: the shortened-window cases need real sleeping (15 s windows):
# roughly 45 s of sleeps in total, plus ~4 boots.
#
# NOT PROBED: no request leaves this machine; the operator's 127.0.0.1:8086
# (production) is never contacted. All bodies are synthetic RE_* values.
# =============================================================================
set -uo pipefail
HERE=$(cd "$(dirname "$0")" && pwd)
# shellcheck source=lib-re.sh
. "$HERE/lib-re.sh"

BASE_URL=${1:?usage: run-rate-limit-experiments.sh <base_url> <outdir> <label>}
OUT=${2:?usage: run-rate-limit-experiments.sh <base_url> <outdir> <label>}
LABEL=${3:?usage: run-rate-limit-experiments.sh <base_url> <outdir> <label>}

BASE_HOST=$(sed -E 's#^https?://##; s#/.*$##; s#:.*$##' <<<"$BASE_URL")
BASE_PORT=$(sed -E 's#^https?://##; s#/.*$##' <<<"$BASE_URL" | sed -nE 's#^[^:]*:([0-9]+)$#\1#p')
BASE_PORT=${BASE_PORT:-18086}
RE_HOST=$BASE_HOST
PORT_RL3600=$((BASE_PORT + 1))
PORT_WIN15=$((BASE_PORT + 2))
PORT_MAX1=$((BASE_PORT + 3))
PORT_XFF=$((BASE_PORT + 4))

re_template_ensure || exit 1

FX="$OUT/fixtures"; mkdir -p "$FX"
printf '%s' '{"username":"admin","email":"re-test@example.test","password":"1234567"}'      > "$FX/pw7.json"
printf '%s' '{"username":"admin","email":"admin@example.test","password":"r3-Str0ng-pw!"}' > "$FX/valid.json"
printf '%s' '{"username":"admin","email":"admin","password":"12345678"}'                     > "$FX/pw8-bademail.json"
printf '%s' '{"username":"admin","email":"admin@localhost","password":"12345678"}'           > "$FX/email-nodot.json"
printf '%s' '{"username":"admin","email":"a b@example.test","password":"12345678"}'          > "$FX/email-forbidden.json"
printf '%s' '{"username":"ad min","email":"re-user@example.test","password":"12345678"}'     > "$FX/username-badchars.json"
printf '%s' '{"username":"admin","email":"re-test@example.test","password":"12345678"'       > "$FX/json-syntax-error.json"
JSON=application/json

# per-posture init: own JSONL, own port, own DB
posture() { # <suffix> <port> <db-suffix> [VAR=value …]
  local sfx="$1" port="$2" dbsfx="$3"; shift 3
  RE_POSTURE="$sfx"; RE_POSTURE_PORT="$port"; RE_POSTURE_DB="re_case_${LABEL}_$dbsfx"
  RE_POSTURE_ENV=("$@")
  re_harness_init "$OUT" "$LABEL-$sfx"
  re_shutdown "$LABEL-$sfx" >/dev/null 2>&1
  re_db_reset "$RE_POSTURE_DB"
  re_boot "$LABEL-$sfx" "$port" "$RE_POSTURE_DB" "${RE_POSTURE_ENV[@]}" \
    || { echo "[expt] boot failed for posture $sfx" >&2; exit 1; }
  RE_CFG_JSON=$(re_cfg_json "$LABEL-$sfx")
  echo "=== posture $sfx port=$port db=$RE_POSTURE_DB env=${RE_POSTURE_ENV[*]:-<release>}"
  echo "    $(cat "$RE_ROOT/run-$LABEL-$sfx/startup.ratelimit.txt" 2>/dev/null)"
  echo "    $(cat "$RE_ROOT/run-$LABEL-$sfx/startup.trustedproxy.txt" 2>/dev/null)"
}
unsafe_fresh() { # fresh DB, same posture/port (keeps the limiter process alive)
  re_shutdown "$LABEL-$RE_POSTURE" >/dev/null 2>&1
  re_db_reset "$RE_POSTURE_DB"
  re_boot "$LABEL-$RE_POSTURE" "$RE_POSTURE_PORT" "$RE_POSTURE_DB" "${RE_POSTURE_ENV[@]}" \
    || { echo "[expt] re-boot failed for posture $RE_POSTURE" >&2; exit 1; }
  RE_CFG_JSON=$(re_cfg_json "$LABEL-$RE_POSTURE")
}

# =============================================================================
# CASE 20 — Retry-After constant across a 3600 s window (release posture)
# =============================================================================
posture rl3600 "$PORT_RL3600" rl3600
re_case_begin retry-after-constant "register 5/3600: consume the budget, sleep 20 s, hit again — Retry-After must still read 3600, not ~3580"
  for i in 1 2 3 4 5 6; do
    re_case_step "post-$i" POST /api/setup "$JSON" "$FX/pw7.json"
  done
  re_case_reading sleep_planned_secs 20
  re_case_reading seconds_at_sleep_start "$SECONDS"
  sleep 20
  re_case_reading seconds_at_second_hit "$SECONDS"
  re_case_step post-after-20s POST /api/setup "$JSON" "$FX/pw7.json"
re_case_end
re_shutdown "$LABEL-rl3600"

# =============================================================================
# CASE 15 — trusted proxy: XFF honoured, first hop wins, shared victim bucket
#   posture: TRUST_PROXY_CIDR=127.0.0.1/32, register 5/3600
# =============================================================================
posture xff "$PORT_XFF" xff "OXICLOUD_TRUST_PROXY_CIDR=127.0.0.1/32"
re_case_begin xff-honoured-trusted "trust 127.0.0.1/32: two distinct leftmost XFF hops must not share a bucket; XFF=127.0.0.1 must share the bare-peer bucket"
  # two spoofed clients, interleaved -> each bucket only ever sees 2 requests
  re_case_step xff-6-1 POST /api/setup "$JSON" "$FX/pw7.json" -H 'X-Forwarded-For: 6.6.6.6, 10.0.0.5'
  re_case_step xff-7-1 POST /api/setup "$JSON" "$FX/pw7.json" -H 'X-Forwarded-For: 7.7.7.7, 10.0.0.5'
  re_case_step xff-6-2 POST /api/setup "$JSON" "$FX/pw7.json" -H 'X-Forwarded-For: 6.6.6.6, 10.0.0.5'
  re_case_step xff-7-2 POST /api/setup "$JSON" "$FX/pw7.json" -H 'X-Forwarded-For: 7.7.7.7, 10.0.0.5'
  # exhaust the bare-peer bucket, then prove XFF=127.0.0.1 lands in the SAME one
  re_case_step peer-1 POST /api/setup "$JSON" "$FX/pw7.json"
  re_case_step xff-peer POST /api/setup "$JSON" "$FX/pw7.json" -H 'X-Forwarded-For: 127.0.0.1'
  re_case_step peer-2 POST /api/setup "$JSON" "$FX/pw7.json"
  re_case_step peer-3 POST /api/setup "$JSON" "$FX/pw7.json"
  re_case_step peer-4 POST /api/setup "$JSON" "$FX/pw7.json"
  re_case_step peer-5 POST /api/setup "$JSON" "$FX/pw7.json"
  # the exhausted bucket is keyed, not global: the other two buckets still live
  re_case_step xff-6-3 POST /api/setup "$JSON" "$FX/pw7.json" -H 'X-Forwarded-For: 6.6.6.6, 10.0.0.5'
  re_case_step xff-7-3 POST /api/setup "$JSON" "$FX/pw7.json" -H 'X-Forwarded-For: 7.7.7.7, 10.0.0.5'
  # and a single-hop victim-shaped value burns a THIRD bucket of its own
  re_case_step victim-1 POST /api/setup "$JSON" "$FX/pw7.json" -H 'X-Forwarded-For: 10.0.0.9'
re_case_end
re_shutdown "$LABEL-xff"

# =============================================================================
# CASE 19 — does a rejected request move the deadline? (max=1, window=15 s)
#   t0  req1 allowed        count=1  ttl-> t15
#   t0  req2 429            count=2  ttl-> t15   (re-insert)
#   t9  req3 429            count=3  ttl-> t24   (re-insert miRVES the deadline)
#   t21 req4 429            21 > 15 (the original deadline) but < 24 -> proof
#   t26 req5 400            26 > 24 -> the refreshed deadline does expire
# =============================================================================
posture max1win15 "$PORT_MAX1" max1win15 \
  "OXICLOUD_RATE_LIMIT_REGISTER_MAX=1" "OXICLOUD_RATE_LIMIT_REGISTER_WINDOW_SECS=15"
re_case_begin ttl-refresh "max=1 window=15 s: rejected requests keep pushing the deadline forward"
  re_case_step t0-req1 POST /api/setup "$JSON" "$FX/pw7.json"
  re_case_step t0-req2 POST /api/setup "$JSON" "$FX/pw7.json"
  re_case_reading t_after_req2 "$SECONDS"
  sleep 9
  re_case_reading t_at_req3 "$SECONDS"
  re_case_step t9-req3 POST /api/setup "$JSON" "$FX/pw7.json"
  sleep 12
  re_case_reading t_at_req4 "$SECONDS"
  re_case_step t21-req4 POST /api/setup "$JSON" "$FX/pw7.json"
  sleep 5
  re_case_reading t_at_req5 "$SECONDS"
  re_case_step t26-req5 POST /api/setup "$JSON" "$FX/pw7.json"
re_case_end

# control: same limiter config, fresh process (empty counter), NO activity inside
# the window -> expiry happens. Boots to clear the counter that case 19 above
# filled — a limiter has no reset endpoint.
unsafe_fresh
re_case_begin ttl-refresh-control "control: same posture, request, idle one full window (17 s), request -> allowed (the window really expires when idle)"
  re_case_step c0-req1 POST /api/setup "$JSON" "$FX/pw7.json"
  re_case_reading c_sleep_planned 17
  sleep 17
  re_case_step c17-req2 POST /api/setup "$JSON" "$FX/pw7.json"
re_case_end

# =============================================================================
# CASE 18 — concurrent rejection burst (max=1, window=15 s)
#   A read-then-write limiter (cache.get + cache.insert) is not atomic: if two
#   requests read the same counter before either writes, more than one request
#   can be SERVED past a max of 1. A count of served>1 proves at least one pair
#   observed the counter simultaneously; it does NOT identify a specific lost
#   update, and a count of exactly 1 does not prove the limiter is atomic.
# =============================================================================
unsafe_fresh   # empty counter: max=1 must start from count=0
re_case_begin concurrent-burst-rejects-8 "max=1: N=8 simultaneous REJECTION posts — how many get served (400) vs 429"
  re_case_burst 8 cbr POST /api/setup "$JSON" "$FX/pw7.json"
re_case_end
unsafe_fresh
re_case_begin concurrent-burst-rejects-16 "max=1: N=16 simultaneous rejection posts (second sample of the same race)"
  re_case_burst 16 cbr16 POST /api/setup "$JSON" "$FX/pw7.json"
re_case_end
re_shutdown "$LABEL-max1win15"

# =============================================================================
# CASE 18b — unconfounded concurrent VALID setups (limiter raised to 100)
#   Isolates the setup handler's one-time claim (INSERT ... ON CONFLICT DO
#   NOTHING) from the rate limiter: every request reaches the handler.
# =============================================================================
posture regwin15 "$PORT_WIN15" regwin15 \
  "OXICLOUD_RATE_LIMIT_REGISTER_MAX=100" "OXICLOUD_RATE_LIMIT_REGISTER_WINDOW_SECS=3600"
re_case_begin concurrent-valid-8-unconfounded "register max=100: N=8 simultaneous VALID setups — expect exactly one 201 and one owner row"
  re_case_burst 8 cv8u POST /api/setup "$JSON" "$FX/valid.json"
re_case_end
unsafe_fresh
re_case_begin concurrent-valid-16-unconfounded "register max=100: N=16 simultaneous VALID setups — expect exactly one 201 and one owner row"
  re_case_burst 16 cv16u POST /api/setup "$JSON" "$FX/valid.json"
re_case_end
# =============================================================================
# COMPRESSION BRACKET — is a body length "raw" or "transferred"?
#   The operator observed a 219-byte body. That number is only interpretable if
#   we know (a) the RAW body length of each 400 branch and (b) what compression
#   does to it. Every step here sends `Accept-Encoding: gzip`, so each recorded
#   pair is "raw N -> transferred M, content-encoding: gzip|(absent)".
#   Posture: register max=100 so all seven steps are served.
# =============================================================================
unsafe_fresh
re_case_begin compression-bracket "Accept-Encoding: gzip on every 400 branch: raw vs transferred length pairs"
  re_case_step ct-415         POST /api/setup text/plain             "$FX/valid.json"            -H 'Accept-Encoding: gzip'
  re_case_step syntax-400     POST /api/setup "$JSON"                "$FX/json-syntax-error.json" -H 'Accept-Encoding: gzip'
  re_case_step pw7-400        POST /api/setup "$JSON"                "$FX/pw7.json"              -H 'Accept-Encoding: gzip'
  re_case_step bademail-400   POST /api/setup "$JSON"                "$FX/pw8-bademail.json"     -H 'Accept-Encoding: gzip'
  re_case_step nodot-400      POST /api/setup "$JSON"                "$FX/email-nodot.json"      -H 'Accept-Encoding: gzip'
  re_case_step forbidden-400  POST /api/setup "$JSON"                "$FX/email-forbidden.json"  -H 'Accept-Encoding: gzip'
  re_case_step badchars-400   POST /api/setup "$JSON"                "$FX/username-badchars.json" -H 'Accept-Encoding: gzip'
re_case_end
re_shutdown "$LABEL-regwin15"

echo "=== experiments done"
ls -l "$OUT"/"$LABEL"-*.jsonl

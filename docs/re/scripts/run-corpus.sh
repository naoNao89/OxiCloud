#!/usr/bin/env bash
# =============================================================================
# run-corpus.sh — fixed-order behavioural case list for the OxiCloud 0.10.0
# RELEASE-POSTURE differential.
#
#   usage: run-corpus.sh <base_url> <outdir> <label>
#          e.g. run-corpus.sh http://127.0.0.1:18086 /data/re-scratch/out/A A
#
# WHAT IT DOES
#   Boots the binary under test itself (RE_BIN) on <base_url>'s host:port, runs
#   the fixed case list below, and appends one JSONL record per case to
#   <outdir>/<label>.jsonl plus the raw bodies/headers/log windows under
#   <outdir>/cases/<case_id>/. Nothing else about the machine state is assumed.
#
#   The script owns the instance because the rate limiter is a per-process
#   in-memory moka cache: to make each case's budget deterministic it
#   shuts the app down, resets the DB from the migrated template and boots
#   again before every case. Case order therefore cannot leak budget or DB
#   state into the next case, and the whole run is byte-for-byte repeatable.
#
# POSTURE
#   Release posture: NO OXICLOUD_RATE_LIMIT_* variable is exported by this
#   script. The effective numbers are read back from the app's own startup line
#   into run-<label>/startup.ratelimit.txt and recorded in every JSONL record's
#   `config` object. Do not export the shortened-window variables here — those
#   belong to run-rate-limit-experiments.sh.
#
# DB STATE
#   Reset (DROP + CREATE ... TEMPLATE <RE_TPL_DB>) before EVERY case. Cases that
#   additionally need pristine state mid-case (10 shared-budget reverse
#   direction, 17b) call re_fresh themselves; those points are noted inline.
#
# CASE LIST (fixed order; ids match the task's list)
#    1 setup-pw7                 2 setup-pw8-bademail       3 setup-email-nodot
#    4 setup-email-forbidden     5 setup-username-badchars  6 setup-json-missing-field
#    7 setup-json-wrong-type     8 setup-json-syntax-error  9 setup-content-type-text
#   10 setup-content-type-absent 11 setup-get               12 rate-limit-6x
#   13 shared-budget             14 xff-ignored-untrusted    16 terminal-201-then-403
#   17 concurrent-valid
#   NOT here (posture-specific, run by run-rate-limit-experiments.sh with ids
#   15, 18, 19, 20 preserved): 15 xff-honoured-trusted (needs
#   OXICLOUD_TRUST_PROXY_CIDR), 18 concurrent-burst-rejects, 19 ttl-refresh,
#   20 retry-after-constant (all need a shortened window to be observable in
#   bounded time).
#
# NOT PROBED: no request ever leaves this machine; no request is sent to the
# operator's 127.0.0.1:8086 instance. All bodies are synthetic RE_* values.
# =============================================================================
set -uo pipefail
HERE=$(cd "$(dirname "$0")" && pwd)
# shellcheck source=lib-re.sh
. "$HERE/lib-re.sh"

BASE_URL=${1:?usage: run-corpus.sh <base_url> <outdir> <label>}
OUT=${2:?usage: run-corpus.sh <base_url> <outdir> <label>}
LABEL=${3:?usage: run-corpus.sh <base_url> <outdir> <label>}

RE_HOST=$(sed -E 's#^https?://##; s#/.*$##; s#:.*$##' <<<"$BASE_URL")
RE_PORT=$(sed -E 's#^https?://##; s#/.*$##' <<<"$BASE_URL" | sed -nE 's#^[^:]*:([0-9]+)$#\1#p')
RE_PORT=${RE_PORT:-18086}
RE_DB=${RE_DB:-re_case_$LABEL}
export RE_HOST RE_PORT RE_DB

re_template_ensure || exit 1
re_harness_init "$OUT" "$LABEL"

# ── fixtures: synthetic only, deterministic bytes ────────────────────────────
FX="$OUT/fixtures"; mkdir -p "$FX"
printf '%s' '{"username":"admin","email":"re-test@example.test","password":"1234567"}'        > "$FX/pw7.json"
printf '%s' '{"username":"admin","email":"admin","password":"12345678"}'                     > "$FX/pw8-bademail.json"
printf '%s' '{"username":"admin","email":"admin@localhost","password":"12345678"}'           > "$FX/email-nodot.json"
printf '%s' '{"username":"admin","email":"a b@example.test","password":"12345678"}'          > "$FX/email-forbidden.json"
printf '%s' '{"username":"ad min","email":"re-user@example.test","password":"12345678"}'     > "$FX/username-badchars.json"
printf '%s' '{"username":"admin","password":"12345678"}'                                     > "$FX/json-missing-field.json"
printf '%s' '{"username":123,"email":"re-user2@example.test","password":"12345678"}'         > "$FX/json-wrong-type.json"
printf '%s' '{"username":"admin","email":"re-test@example.test","password":"12345678"'       > "$FX/json-syntax-error.json"
printf '%s' '{"username":"admin","email":"admin@example.test","password":"r3-Str0ng-pw!"}'   > "$FX/valid.json"
printf '%s' '{"email":"re-reg@example.test","password":"r3-Str0ng-pw!"}'                     > "$FX/register.json"

JSON=application/json

# shut down, reset the DB, boot again — deterministic budget + state per case
re_fresh() {
  re_shutdown "$LABEL" >/dev/null 2>&1
  re_db_reset "$RE_DB" || { echo "[corpus] db reset failed for $RE_DB — read the container log; this is NOT a boot failure" >&2; exit 1; }
  re_boot "$LABEL" "$RE_PORT" "$RE_DB" || { echo "[corpus] boot failed for $LABEL" >&2; exit 1; }
  RE_CFG_JSON=$(re_cfg_json)
}

echo "=== corpus label=$LABEL url=$BASE_URL db=$RE_DB bin=$RE_BIN" | tee "$OUT/$LABEL.run.txt"
echo "=== effective limiter: $(cat "$RE_ROOT/run-$LABEL/startup.ratelimit.txt" 2>/dev/null)" | tee -a "$OUT/$LABEL.run.txt"
echo "=== trusted proxy:    $(cat "$RE_ROOT/run-$LABEL/startup.trustedproxy.txt" 2>/dev/null)" | tee -a "$OUT/$LABEL.run.txt"

# ── 1..5 — setup-body validation failures ────────────────────────────────────
re_fresh
re_case_begin setup-pw7 "7-char password: expect 400, init stays false, claim not left behind"
  re_case_step post-pw7 POST /api/setup "$JSON" "$FX/pw7.json"
re_case_end

re_fresh
re_case_begin setup-pw8-bademail "8-char pw, email 'admin' (no @): expect 400. CANDIDATE branch for the operator-observed 219-byte body"
  re_case_step post-bademail POST /api/setup "$JSON" "$FX/pw8-bademail.json"
re_case_end

re_fresh
re_case_begin setup-email-nodot "email admin@localhost: expect 400"
  re_case_step post-nodot POST /api/setup "$JSON" "$FX/email-nodot.json"
re_case_end

re_fresh
re_case_begin setup-email-forbidden "email 'a b@example.test' > 256-byte raw body: record whether the response is gzip-compressed and the transferred size"
  re_case_step post-forbidden POST /api/setup "$JSON" "$FX/email-forbidden.json"
  re_case_step post-forbidden-gzip POST /api/setup "$JSON" "$FX/email-forbidden.json" -H 'Accept-Encoding: gzip'
re_case_end

re_fresh
re_case_begin setup-username-badchars "username 'ad min' > 256-byte raw body: compression threshold probe"
  re_case_step post-badchars POST /api/setup "$JSON" "$FX/username-badchars.json"
  re_case_step post-badchars-gzip POST /api/setup "$JSON" "$FX/username-badchars.json" -H 'Accept-Encoding: gzip'
re_case_end

# ── 6..10 — extractor / content-type branch ──────────────────────────────────
re_fresh
re_case_begin setup-json-missing-field "body without email: expect 422 (axum Json extractor)"
  re_case_step post-missing POST /api/setup "$JSON" "$FX/json-missing-field.json"
re_case_end

re_fresh
re_case_begin setup-json-wrong-type "username as number: expect 422"
  re_case_step post-wrongtype POST /api/setup "$JSON" "$FX/json-wrong-type.json"
re_case_end

re_fresh
re_case_begin setup-json-syntax-error "truncated JSON: expect 400"
  re_case_step post-syntax POST /api/setup "$JSON" "$FX/json-syntax-error.json"
re_case_end

re_fresh
re_case_begin setup-content-type-text "valid JSON body but Content-Type: text/plain: expect 415"
  re_case_step post-text POST /api/setup text/plain "$FX/valid.json"
re_case_end

re_fresh
re_case_begin setup-content-type-absent "valid JSON body, Content-Type header removed: expect 415"
  re_case_step post-noct POST /api/setup ABSENT "$FX/valid.json"
re_case_end

# ── 11 — does the limiter run before routing/extraction? ─────────────────────
# GET is not a method /api/setup serves, so a 405 proves routing rejected it.
# If the register limiter also counted the GET, then only FOUR of the five
# following POSTs can pass the limiter (5 total slots: GET + 4 POSTs) and the
# 5th POST answers 429. If the GET did not count, all five POSTs pass.
re_fresh
re_case_begin setup-get "GET /api/setup (405) and whether it consumed a register slot"
  re_case_step get-setup GET /api/setup ABSENT -
  re_case_step post-1 POST /api/setup "$JSON" "$FX/pw7.json"
  re_case_step post-2 POST /api/setup "$JSON" "$FX/pw7.json"
  re_case_step post-3 POST /api/setup "$JSON" "$FX/pw7.json"
  re_case_step post-4 POST /api/setup "$JSON" "$FX/pw7.json"
  re_case_step post-5 POST /api/setup "$JSON" "$FX/pw7.json"
re_case_end

# ── 12 — six rejections, expect the 6th to be a 429 with Retry-After ─────────
re_fresh
re_case_begin rate-limit-6x "six setup-pw7 POSTs: 5 pass the limiter (400), the 6th must be 429 with Retry-After"
  for i in 1 2 3 4 5 6; do
    re_case_step "post-$i" POST /api/setup "$JSON" "$FX/pw7.json"
  done
re_case_end

# ── 13 — /api/auth/register and /api/setup share one budget, both directions ─
re_fresh
re_case_begin shared-budget "exhaust with register then setup -> 429; then re_fresh and the reverse direction"
  for i in 1 2 3 4 5; do
    re_case_step "register-$i" POST /api/auth/register "$JSON" "$FX/register.json"
  done
  re_case_step setup-after-register POST /api/setup "$JSON" "$FX/pw7.json"
  re_fresh   # pristine DB + fresh limiter for the reverse direction
  for i in 1 2 3 4 5; do
    re_case_step "setup-$i" POST /api/setup "$JSON" "$FX/pw7.json"
  done
  re_case_step register-after-setup POST /api/auth/register "$JSON" "$FX/register.json"
re_case_end

# ── 14 — untrusted XFF must be ignored (trust list empty in release posture) ─
# Different X-Forwarded-For on each request; all six must land in ONE bucket:
# requests 1..5 pass, the 6th is 429, and the limiter's own log line names the
# bucket it used (`ip=` field) — that is the key evidence, not the statuses.
re_fresh
re_case_begin xff-ignored-untrusted "empty trust list: X-Forwarded-For must not split the bucket"
  re_case_step xff-1 POST /api/setup "$JSON" "$FX/pw7.json" -H 'X-Forwarded-For: 6.6.6.6'
  re_case_step xff-2 POST /api/setup "$JSON" "$FX/pw7.json" -H 'X-Forwarded-For: 7.7.7.7'
  re_case_step xff-3 POST /api/setup "$JSON" "$FX/pw7.json" -H 'X-Forwarded-For: 8.8.8.8'
  re_case_step xff-4 POST /api/setup "$JSON" "$FX/pw7.json" -H 'X-Forwarded-For: 9.9.9.9'
  re_case_step xff-5 POST /api/setup "$JSON" "$FX/pw7.json"
  re_case_step xff-6 POST /api/setup "$JSON" "$FX/pw7.json"
re_case_end

# ── 16 — the terminal state: 201 once, then 403 forever ─────────────────────
re_fresh
re_case_begin terminal-201-then-403 "valid setup -> 201; repeat -> 403 SystemAlreadyInitialized; state must hold exactly one owner"
  re_case_step valid-1 POST /api/setup "$JSON" "$FX/valid.json"
  re_case_step valid-2 POST /api/setup "$JSON" "$FX/valid.json"
  re_case_step status GET /api/auth/status ABSENT -
re_case_end

# ── 17 — concurrent valid setups: exactly one 201, exactly one owner ────────
re_fresh
re_case_begin concurrent-valid-2 "N=2 simultaneous valid setup POSTs on a fresh DB"
  re_case_burst 2 cv2 POST /api/setup "$JSON" "$FX/valid.json"
re_case_end

# NB: N=8 under the release posture is confounded — max=5 means requests 6..8
# can be 429 before ever reaching the claim. The unconfounded N=8/16 run (raised
# limiter, isolating the claim's INSERT ... ON CONFLICT) is case 18b in
# run-rate-limit-experiments.sh. Both are recorded.
re_fresh
re_case_begin concurrent-valid-8 "N=8 simultaneous valid setup POSTs, release posture (3 of them may be 429: documented confound)"
  re_case_burst 8 cv8 POST /api/setup "$JSON" "$FX/valid.json"
re_case_end

re_shutdown "$LABEL" >/dev/null 2>&1
echo "=== corpus done: $RE_JSONL ($(wc -l < "$RE_JSONL") records)" | tee -a "$OUT/$LABEL.run.txt"

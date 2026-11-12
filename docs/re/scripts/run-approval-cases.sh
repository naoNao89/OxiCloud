#!/usr/bin/env bash
# =============================================================================
# run-approval-cases.sh — the three RE cases a reviewer requires as a CONDITION
# of approving patch `0002` (setup's own rate-limit budget).
#
#   usage: run-approval-cases.sh <base_url> <outdir> <label>
#          e.g. run-approval-cases.sh http://127.0.0.1:18086 \
#                   /data/re-scratch/out-apv apv-a
#
# It is a CASE LIST on top of docs/re/scripts/lib-re.sh — the same boot, DB,
# Postgres-window and JSONL-emitter plumbing as run-corpus.sh, not a second
# harness. The trim rule is the harness's: scalars + state + per-step SQL
# summaries inline, raw log windows referenced by {path, lines, sha256}.
#
# TWO POSTURES (each number is labelled with the one that produced it)
#   <label>.jsonl              release posture (no OXICLOUD_RATE_LIMIT_* var)
#       setup-validation-then-valid        a REJECTED setup then a VALID one on
#                                          the SAME instance, no DB reset between
#                                          (#677 claim-release, in sequence)
#       setup-after-owner-across-restart   owner created -> app RESTARTED with the
#                                          DB intact -> POST /api/setup must be
#                                          403. The only case that separates the
#                                          RAM limiter (cleared by the restart)
#                                          from the DB-backed initialised state.
#   <label>-setupbudget.jsonl  OXICLOUD_RATE_LIMIT_SETUP_{MAX=1,WINDOW_SECS=15}
#                              and REGISTER_{MAX=1,WINDOW_SECS=15} — BOTH budgets
#                              pinned, because on v0.10.0/HEAD-without-0002 the
#                              setup route is served by the REGISTER limiter and
#                              SETUP_* does not exist (silently ignored). Same
#                              posture on both binaries, so the comparison is
#                              apples to apples.
#       setup-get-spends-setup-budget      GET /api/setup (405) then a POST:
#                                          does the non-POST spend a slot? and
#                                          WHICH budget did it spend? (the register
#                                          probe right after answers that: on a
#                                          tree where they share, register is also
#                                          429; where 0002 split them, register is
#                                          served) — RECORDED, NOT CHANGED: whether
#                                          a 405 should spend budget is a policy
#                                          question, not a defect this wave fixes.
#
# NOT PROBED: no request leaves this machine; the operator's 127.0.0.1:8086
# (production) is never contacted. Synthetic RE_* values only.
# =============================================================================
set -uo pipefail
HERE=$(cd "$(dirname "$0")" && pwd)
# shellcheck source=lib-re.sh
. "$HERE/lib-re.sh"

BASE_URL=${1:?usage: run-approval-cases.sh <base_url> <outdir> <label>}
OUT=${2:?usage: run-approval-cases.sh <base_url> <outdir> <label>}
LABEL=${3:?usage: run-approval-cases.sh <base_url> <outdir> <label>}

RE_HOST=$(sed -E 's#^https?://##; s#/.*$##; s#:.*$##' <<<"$BASE_URL")
RE_PORT=$(sed -E 's#^https?://##; s#/.*$##' <<<"$BASE_URL" | sed -nE 's#^[^:]*:([0-9]+)$#\1#p')
RE_PORT=${RE_PORT:-18086}
RE_DB=${RE_DB:-re_case_$LABEL}
export RE_HOST RE_PORT RE_DB

re_template_ensure || exit 1
mkdir -p "$OUT"

# ── fixtures: synthetic only, deterministic bytes (same as the corpus) ──────
FX="$OUT/fixtures"; mkdir -p "$FX"
printf '%s' '{"username":"admin","email":"re-test@example.test","password":"1234567"}'      > "$FX/pw7.json"
printf '%s' '{"username":"admin","email":"admin@example.test","password":"r3-Str0ng-pw!"}' > "$FX/valid.json"
printf '%s' '{"email":"re-reg@example.test","password":"r3-Str0ng-pw!"}'                   > "$FX/register.json"
JSON=application/json

re_fresh() { # shutdown, reset the DB, boot again — release posture
  re_shutdown "$LABEL" >/dev/null 2>&1
  re_db_reset "$RE_DB"
  re_boot "$LABEL" "$RE_PORT" "$RE_DB" || { echo "[apv] boot failed for $LABEL" >&2; exit 1; }
  RE_CFG_JSON=$(re_cfg_json)
}

echo "=== approval cases label=$LABEL url=$BASE_URL db=$RE_DB bin=$RE_BIN" | tee "$OUT/$LABEL.run.txt"
echo "=== effective limiter: $(cat "$RE_ROOT/run-$LABEL/startup.ratelimit.txt" 2>/dev/null)" | tee -a "$OUT/$LABEL.run.txt"

# =============================================================================
# PHASE 1 — release posture: cases 1 and 2
# =============================================================================
re_harness_init "$OUT" "$LABEL"

# ── 1 — rejected setup, then a VALID setup on the same instance ─────────────
re_fresh
re_case_begin setup-validation-then-valid \
  "#677 in sequence: a 400 validation failure releases the initialization claim, so the NEXT valid POST on the same instance must answer 201 with exactly one owner (role owner). No DB reset between steps."
  re_case_step pw7 POST /api/setup "$JSON" "$FX/pw7.json"
  re_case_step valid-after-failure POST /api/setup "$JSON" "$FX/valid.json"
re_case_end

# ── 2 — owner created, app RESTARTED with the DB intact, setup again ────────
re_fresh
re_case_begin setup-after-owner-across-restart \
  "owner created (201) -> app restarted WITHOUT wiping the DB -> POST /api/setup must be 403 SystemAlreadyInitialized with si_rows=1 and exactly one owner. An empty RAM limiter after the restart rules out a 429 as the explanation."
  re_case_step valid-1 POST /api/setup "$JSON" "$FX/valid.json"
  # restart: same label, same DB, same storage — only the process goes away
  re_shutdown "$LABEL" >/dev/null 2>&1
  re_boot "$LABEL" "$RE_PORT" "$RE_DB" || { echo "[apv] reboot failed" >&2; exit 1; }
  re_case_reading restart_kept_db by-design   # assertion by the script (no re_db_reset between the two steps); the citable facts are state_after and the 403 from a fresh process
  re_case_step after-restart POST /api/setup "$JSON" "$FX/valid.json"
re_case_end

re_shutdown "$LABEL" >/dev/null 2>&1
echo "=== phase 1 done: $OUT/$LABEL.jsonl ($(wc -l < "$OUT/$LABEL.jsonl" 2>/dev/null) records)" | tee -a "$OUT/$LABEL.run.txt"

# =============================================================================
# PHASE 2 — its own posture: case 3 (does a 405 spend budget, and whose?)
# =============================================================================
SB="$LABEL-setupbudget"
POSTURE=("OXICLOUD_RATE_LIMIT_SETUP_MAX=1" "OXICLOUD_RATE_LIMIT_SETUP_WINDOW_SECS=15"
         "OXICLOUD_RATE_LIMIT_REGISTER_MAX=1" "OXICLOUD_RATE_LIMIT_REGISTER_WINDOW_SECS=15")
re_shutdown "$SB" >/dev/null 2>&1
re_db_reset "$RE_DB"
re_boot "$SB" "$RE_PORT" "$RE_DB" "${POSTURE[@]}" || { echo "[apv] boot failed (setupbudget)" >&2; exit 1; }
# The `config` object of every record is read back from THIS instance's env.sh
# (lib-re.sh `re_cfg_json`), whose allow-list predates 0002 and therefore does not
# name OXICLOUD_RATE_LIMIT_SETUP_*. So the record carries the posture twice: the
# allow-listed vars via `config`, and the four posture vars verbatim via `readings`
# (the harness's free-form channel), so a reader can always tell which posture
# produced a number without leaving the JSONL.
RE_CFG_JSON=$(re_cfg_json)
echo "=== posture setupbudget: $(tr '\n' ' ' < "$RE_ROOT/run-$SB/env.sh" | sed -E 's/postgres:[^ ]*/postgres:RE_PGPASS/')" | tee -a "$OUT/$LABEL.run.txt"
echo "=== config recorded for this posture: $RE_CFG_JSON" | tee -a "$OUT/$LABEL.run.txt"
re_harness_init "$OUT" "$SB"
re_case_begin setup-get-spends-setup-budget \
  "SETUP_MAX=1 + REGISTER_MAX=1 (both pinned): GET /api/setup (405) must consume one slot, so the following POST is 429; the register probe immediately after says WHICH budget was spent (shared => 429, 0002's split => served). Recorded, not changed: whether a non-POST should spend budget is a policy question."
  re_case_reading posture_setup_max 1
  re_case_reading posture_setup_window_secs 15
  re_case_reading posture_register_max 1
  re_case_reading posture_register_window_secs 15
  re_case_reading config_from_env_sh "$RE_CFG_JSON"
  re_case_step get-setup GET /api/setup ABSENT -
  re_case_step post-after-get POST /api/setup "$JSON" "$FX/pw7.json"
  re_case_step register-probe POST /api/auth/register "$JSON" "$FX/register.json"
  re_case_step post-setup-again POST /api/setup "$JSON" "$FX/pw7.json"
re_case_end
re_shutdown "$SB" >/dev/null 2>&1
echo "=== phase 2 done: $OUT/$SB.jsonl ($(wc -l < "$OUT/$SB.jsonl" 2>/dev/null) records)" | tee -a "$OUT/$LABEL.run.txt"

# =============================================================================
# PHASE 3 — its own posture: case 4, the deadline question on the COMPOSITION
#   (0001 + 0002), which the c1l-only trace of `ttl-refresh` cannot answer.
#
#   Posture: SETUP_{MAX=1,WINDOW_SECS=15} and REGISTER_{MAX=1,WINDOW_SECS=15}.
#   Both pinned on purpose, so ONE posture is meaningful on both binaries:
#     * on c1 the setup route is served by SETUP's limiter (0002) — the primary row;
#     * on A `SETUP_*` does not exist and is ignored, so the register limiter
#       serves the route: that row is the OLD (re-arming, constant-Retry-After)
#       behaviour, a contrast, NOT a failure of the case.
# =============================================================================
SD="$LABEL-setupdeadline"
DPOSTURE=("OXICLOUD_RATE_LIMIT_SETUP_MAX=1" "OXICLOUD_RATE_LIMIT_SETUP_WINDOW_SECS=15"
          "OXICLOUD_RATE_LIMIT_REGISTER_MAX=1" "OXICLOUD_RATE_LIMIT_REGISTER_WINDOW_SECS=15")
re_shutdown "$SD" >/dev/null 2>&1
re_db_reset "$RE_DB"
re_boot "$SD" "$RE_PORT" "$RE_DB" "${DPOSTURE[@]}" || { echo "[apv] boot failed (setupdeadline)" >&2; exit 1; }
RE_CFG_JSON=$(re_cfg_json)
echo "=== posture setupdeadline: $(tr '\n' ' ' < "$RE_ROOT/run-$SD/env.sh" | sed -E 's/postgres:[^ ]*/postgres:RE_PGPASS/')" | tee -a "$OUT/$LABEL.run.txt"
re_harness_init "$OUT" "$SD"
re_case_begin setup-deadline-on-c1 \
  "The deadline condition on the APPROVED COMPOSITION (0001+0002), mirroring ttl-refresh: req1 400 (window opens, closes at +15 s) -> req2 429 with ra = remaining -> sleep 9 -> req3 429 ra smaller -> sleep 12 -> req4 400 (window closed on schedule) -> sleep 5 -> req5 429 ra smaller again. On A the same posture exercises the OLD re-arming path (SETUP_* is inert there and the register limiter serves the route)."
  re_case_reading posture_setup_max 1
  re_case_reading posture_setup_window_secs 15
  re_case_reading posture_register_max 1
  re_case_reading posture_register_window_secs 15
  re_case_reading config_from_env_sh "$RE_CFG_JSON"
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
re_shutdown "$SD" >/dev/null 2>&1
echo "=== phase 3 done: $OUT/$SD.jsonl ($(wc -l < "$OUT/$SD.jsonl" 2>/dev/null) records)" | tee -a "$OUT/$LABEL.run.txt"
echo "=== approval cases done for $LABEL" | tee -a "$OUT/$LABEL.run.txt"
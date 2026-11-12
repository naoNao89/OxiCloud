#!/usr/bin/env bash
# shellcheck shell=bash
# =============================================================================
# lib-re.sh — shared driver for the OxiCloud 0.10.0 behavioural RE harness.
#
# Source it; do not execute it:
#     . "$(dirname "$0")/lib-re.sh"
#
# It owns the sandbox layout, the Postgres statement-log oracle, app
# boot/shutdown, DB reset/clone, state probes, the per-case recorder and the
# JSONL emitter. `run-corpus.sh` and `run-rate-limit-experiments.sh` are thin
# case lists on top of it.
#
# SSOT: the SAME scripts must be byte-for-byte re-runnable against a different
# 0.10.0 build (source build, patched build) so the emitted JSONL can be
# diffed. Nothing in here may depend on which binary is under test.
#
# -----------------------------------------------------------------------------
# SANDBOX LAYOUT (all under the RE_* env vars below; override if you must)
#   $RE_BIN                     the released 0.10.0 x86_64-musl binary
#   $RE_ROOT/pglog/postgresql.log   PG statement log (log_statement=all)
#   $RE_ROOT/run-<label>/{env.sh,app.log,app.pid,boots.log,startup.*.txt}
#   $RE_DATA/<label>/           per-instance storage dir (secrets land here)
#   <outdir>/cases/<case_id>/   per-case raw evidence
#
# -----------------------------------------------------------------------------
# JSONL RECORD SCHEMA (one line per case, one file: <outdir>/<label>.jsonl)
#
#   case_id, label, note
#   db_now, db_now_end      the DB's own clock (ISO-8601 UTC), opening/closing the
#                           case window so PG statement timestamps can be aligned
#                           against the database rather than the host clock
#   config         object   rate-limit / trust-proxy env vars in effect
#   state_before   object   {users, owners, si_rows, si_value, si_updated_by}
#   state_after    object   same shape, read after the case's requests
#   pg_case_log_line_count  int    lines of other connection activity in the case
#   pg_case_log_window      object {path, lines, sha256} for that window
#                                  and minus every line already reported in a
#                                  step window, i.e. other connection activity
#                                  inside the case window (background scheduler
#                                  jobs). Non-empty here = the case window saw
#                                  DB activity that is NOT attributable to the
#                                  request.
#   readings       object   free-form string readings (timing experiments)
#   steps          array    one entry per HTTP request, in the order the harness
#                           dispatched them (for a burst: launch order).
#                           wall_epoch_us gives the observed dispatch time.
#                           NOTE: for burst steps the per-step app/PG log WINDOWS
#                           overlap (the requests really are simultaneous), so
#                           only the request-id-scoped fields below are
#                           per-request: handler_ran, app_log_lines,
#                           app_sql_statement_count, app_sql_statement_summaries.
#       step, name
#       request.{method,path,content_type,extra_args,body_file,body_len}
#       response.{status,content_type,content_encoding,content_length,
#                 retry_after,x_request_id}
#       body_file, body_len         = bytes that crossed the socket (size_download)
#       body_file_bytes             = bytes saved on disk
#       body_decoded_len            = length after undoing content-encoding —
#                                     the number to compare against "raw N bytes"
#       body_decoded_source         = gzip|deflate|br|zstd|identity|<reason>
#       body_decoded_file           = decoded copy, when the coding was undone
#       elapsed_ms, ttfb_ms   curl %{time_total}/%{time_starttransfer} * 1000
#       wall_epoch_us, wall_epoch_us_end   wall clock (ordering only)
#       handler_ran           bool — "Setup admin request received" in window
#       app_log_lines         lines from the step window matching this step's
#                             x-request-id or a known marker, as a STABLE
#                             projection: tracing span prefix and timing fields
#                             (latency_ms=, elapsed=, …) removed, timestamp
#                             removed, sqlx lines omitted (they are in
#                             app_sql_statement_summaries). Two builds diff
#                             cleanly on this.
#       resolved_client_ip    the client IP the request span resolved, with the
#                             TCP port stripped (this is what the trusted-proxy
#                             path picked: the leftmost X-Forwarded-For hop when
#                             the peer is trusted, else the bare peer IP)
#       limiter_ip            the key the limiter logged on a 429
#       app_log_window_lines  raw count in the step window
#       app_log_file          raw step window path (unfiltered)
#       app_sql_statement_count      number of sqlx queries logged INSIDE this
#                                    request's span — the per-request DB-I/O
#                                    oracle (a 429 must show 0 here)
#       app_sql_statement_summaries  their `summary=` strings, in order
#       pg_log_line_count            LINES of the Postgres log inside this step's
#                                    window. NOT statements attributable to the
#                                    request: background scheduler jobs (e.g. the
#                                    tree_etag_dirty drain) write into any open
#                                    window, so this can be large on a request
#                                    that issued no SQL. The per-request oracle
#                                    is app_sql_statement_count/_summaries.
#       pg_log_window                {path, lines, sha256} — the raw PG window
#                                    stays on the sandbox host, not in the
#                                    committed JSONL (RE_FULL_LOGS=1 inlines it)
#
# TRIMMING RULE (why the committed artifact is ~10x smaller than the live one):
# every scalar field, state_before/after and the per-request SQL summaries are
# always inline; the RAW log line arrays are referenced by {path, lines, sha256}
# instead of being pasted, because 5 postures x N builds of pasted windows is
# ~10 MB of JSONL in a PR. app_log_lines is the ONE exception kept inline: it is
# a stable projection, ~40 KB for a whole corpus, and it carries the reason behind
# handler_ran (handler markers, claim_released, Rate limit exceeded).
#
# NONDETERMINISTIC FIELDS — exclude these from a cross-run / cross-build diff:
#   state_before.si_updated_by, state_after.si_updated_by   (random admin UUID)
#   steps[].response.x_request_id                           (random request id)
#   steps[].wall_epoch_us, wall_epoch_us_end, elapsed_ms, ttfb_ms, db_now*  (time)
#   the WINNER POSITION among simultaneous burst steps      (race outcome)
#   pg_log_window.sha256 / pg_log_window.lines, pg_case_log_* (background churn)
# Everything else is deterministic per step: status, content-type, content_*,
# retry_after, body_len, body_file_bytes, body_decoded_len, handler_ran,
# app_sql_statement_count, app_sql_statement_summaries, resolved_client_ip,
# limiter_ip, app_log_lines, state_*.{users,owners,si_rows,si_value}.
# Verified by docs/re/scripts/diff-labels.py against the DELIVERED files:
#   cases identical (field by field)      : 20
#   cases with same status multiset only  : 6    (the simultaneous bursts)
#   cases whose status multiset differs   : 0
# Reproduce: python3 docs/re/scripts/diff-labels.py docs/re/evidence
# It also normalises the two HOST-varying strings the app writes into its own
# log lines (the free-disk-space number, and the UUIDs a run mints for the user,
# personal drive, calendar and address book) — without those two the comparison
# would report noise on every user-creating case.
#
# BACKGROUND LOAD IS PART OF THE ENVIRONMENT. This sandbox is shared, so the
# label a run is given must say what else was running. Delivered convention
# (see docs/re/runtime-evidence.md §A):
#   label `A-*`         the QUIET pass  — DB clock 14:47:22 → 14:49:26, after two
#                       unrelated `driver-libfuzzer` processes (~2 of 4 vCPUs,
#                       ~54 % box-busy by /proc/stat jiffies) were removed.
#                       PRIMARY: every number quoted in the report is from here.
#   label `A-loaded-*`  the pass that STARTED under that load — DB clock
#                       14:40:36 → 14:42:41; the load was removed at ≈14:41, so
#                       its corpus part is contended and its later postures may
#                       not have been. Kept as a cross-check only.
# Cases whose assertion is an elapsed wall-clock value (ttl-refresh,
# retry-after-constant) or a race outcome (the concurrent-* bursts) are reported
# from the quiet pass with the loaded numbers beside them. Before running, note
# which state the box is in — and give the run a label that says it.
#
# TIMING: elapsed_ms/ttfb_ms come from curl's own monotonic timer — that is the
# elapsed_ms of record. wall_epoch_us is wall clock, used only to reconstruct
# ordering/interleaving across steps; never used to compute a duration.
# No subprocess-spawning clock is used in the hot path (429s answer in <2 ms).
# =============================================================================

# ── tunables ─────────────────────────────────────────────────────────────────
RE_ROOT=${RE_ROOT:-/data/re-scratch}
RE_DATA=${RE_DATA:-/data/re-data}
RE_BIN=${RE_BIN:-$RE_ROOT/oxicloud-0.10.0-x86_64-unknown-linux-musl/oxicloud}
RE_PG_CONTAINER=${RE_PG_CONTAINER:-re-pg}
RE_PG_HOST=${RE_PG_HOST:-127.0.0.1}
RE_PG_PORT=${RE_PG_PORT:-55432}
RE_PG_USER=${RE_PG_USER:-re_user}
# Throwaway value for the loopback-only sandbox container — NEVER a credential.
# Nothing committed under docs/ may contain it: run-<label>/env.sh is written
# with RE_PGPASS in its place and log lines are quoted through re_redact.
RE_PG_PASS=${RE_PG_PASS:-RE_pgpass_x9}
RE_PGLOG=${RE_PGLOG:-$RE_ROOT/pglog/postgresql.log}      # host view
RE_PGLOG_IN=${RE_PGLOG_IN:-/pglog/postgresql.log}        # container view
RE_TPL_DB=${RE_TPL_DB:-re_tpl}
RE_HOST=${RE_HOST:-127.0.0.1}
RE_RUST_LOG=${RE_RUST_LOG:-debug}
RE_PROBE_TAG=${RE_PROBE_TAG:-RE_PROBE}
RE_SETTLE=${RE_SETTLE:-0.20}      # let app+PG flush before slicing log windows
RE_CURL=${RE_CURL:-curl}

# ── redaction ────────────────────────────────────────────────────────────────
re_redact() {
  sed -E \
    -e "s/${RE_PG_PASS}/RE_PGPASS/g" \
    -e 's#postgres://[^:/@]+:[^@]*@#postgres://RE_PGUSER:RE_PGPASS@#g' \
    -e 's/[A-Za-z0-9_-]{20,}\.[A-Za-z0-9_-]{20,}\.[A-Za-z0-9_-]{20,}/RE_JWT/g'
}

# ── Postgres plumbing ────────────────────────────────────────────────────────
# Every probe carries RE_PROBE_TAG so the harness can filter its own statements
# out of the reported statement windows (the app queries the same tables).
re_psql() { # <db> <sql>
  docker exec -i "$RE_PG_CONTAINER" psql -U "$RE_PG_USER" -d "$1" -tAX -v ON_ERROR_STOP=1 \
    -c "/* $RE_PROBE_TAG */ $2"
}

re_pg_ready() { docker exec -i "$RE_PG_CONTAINER" pg_isready -U "$RE_PG_USER" >/dev/null 2>&1; }

# Canonical state probe. One line, pipe separated:
#   users|owners|si_rows|si_value|si_updated_by
# si_* describe auth.admin_settings WHERE key='system_initialized' — the row the
# setup handler's claim inserts with updated_by = Uuid::nil().
re_state() { # <db>
  re_psql "$1" "SELECT
      (SELECT count(*) FROM auth.users)::text || '|' ||
      (SELECT count(*) FROM auth.users WHERE role = 'owner')::text || '|' ||
      (SELECT count(*) FROM auth.admin_settings WHERE key = 'system_initialized')::text || '|' ||
      COALESCE((SELECT value FROM auth.admin_settings WHERE key = 'system_initialized'), '') || '|' ||
      COALESCE((SELECT updated_by::text FROM auth.admin_settings WHERE key = 'system_initialized'), '')"
}

# The database's own wall clock — anchor for the PG statement window.
re_db_now() { # <db>
  re_psql "$1" "SELECT to_char(now() AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS.MS\"Z\"')"
}

# ── DB lifecycle ─────────────────────────────────────────────────────────────
re_db_drop() { # <db>
  docker exec -i "$RE_PG_CONTAINER" psql -U "$RE_PG_USER" -d postgres -tAX -c \
    "SELECT pg_terminate_backend(pid) FROM pg_stat_activity WHERE datname = '$1' AND pid <> pg_backend_pid()" >/dev/null
  docker exec -i "$RE_PG_CONTAINER" psql -U "$RE_PG_USER" -d postgres -tAX -c \
    "DROP DATABASE IF EXISTS \"$1\"" >/dev/null 2>&1
}

re_db_clone() { # <db> — clone the migrated template
  docker exec -i "$RE_PG_CONTAINER" psql -U "$RE_PG_USER" -d postgres -tAX -v ON_ERROR_STOP=1 -c \
    "CREATE DATABASE \"$1\" TEMPLATE \"$RE_TPL_DB\"" >/dev/null
}

# Fresh, migrated, pristine DB — use where a case needs pristine state.
re_db_reset() { # <db>
  # Both steps matter and both must be able to fail the caller. A DROP that fails
  # leaves the clone with a live target; a CLONE that fails used to be swallowed
  # entirely by callers without `set -e`, surfacing later as a bare "boot failed"
  # that named the wrong cause.
  re_db_drop "$1" || return 1
  re_db_clone "$1" || return 1
}

re_template_ensure() {
  if [ "$(re_psql postgres "SELECT count(*) FROM pg_database WHERE datname='$RE_TPL_DB'")" = "1" ]; then
    return 0
  fi
  echo "[lib-re] creating migration template DB $RE_TPL_DB (app migrations only)" >&2
  docker exec -i "$RE_PG_CONTAINER" psql -U "$RE_PG_USER" -d postgres -tAX -c \
    "CREATE DATABASE \"$RE_TPL_DB\"" >/dev/null
  # The template instance is NOT the caller's own: re_boot assigns the globals
  # RE_LABEL/RE_PORT/RE_DB, so booting it in-process silently rewrites the
  # caller's port and database name. Where the template already exists the guard
  # above returns early and the leak is invisible; on a fresh container -- CI's
  # service container, which has no volume -- the caller then resets "re_tpl"
  # against itself ("CREATE DATABASE re_tpl TEMPLATE re_tpl" -> template does not
  # exist) and the first case dies as a boot failure. Run it in a subshell so the
  # caller keeps its own RE_DB/RE_PORT.
  ( re_boot tpl 18099 "$RE_TPL_DB" ) || { echo "[lib-re] template boot failed" >&2; return 1; }
  re_shutdown tpl
  local n
  n=$(re_psql "$RE_TPL_DB" "SELECT count(*) FROM _sqlx_migrations WHERE success")
  echo "[lib-re] template migrations applied: $n" >&2
}

# ── app lifecycle ────────────────────────────────────────────────────────────
# re_boot <label> <port> <db> [VAR=value ...]
# Writes run-<label>/env.sh (password redacted) — that file is the recorded env.
re_boot() {
  local label="$1" port="$2" db="$3"; shift 3
  RE_LABEL="$label"; RE_PORT="$port"; RE_DB="$db"
  local run="$RE_ROOT/run-$label"
  mkdir -p "$run" "$RE_DATA/$label"
  {
    echo "# effective environment — password replaced by RE_PGPASS before record"
    echo "OXICLOUD_DB_CONNECTION_STRING=postgres://$RE_PG_USER:RE_PGPASS@$RE_PG_HOST:$RE_PG_PORT/$db"
    echo "OXICLOUD_STORAGE_PATH=$RE_DATA/$label"
    echo "OXICLOUD_SERVER_HOST=$RE_HOST"
    echo "OXICLOUD_SERVER_PORT=$port"
    echo "RUST_LOG=$RE_RUST_LOG"
    local a; for a in "$@"; do echo "$a"; done
  } > "$run/env.sh"
  {
    echo "=== boot $(date -u +%FT%TZ) label=$label port=$port db=$db"
    sed -e 's/^/    /' "$run/env.sh"
  } >> "$run/boots.log"

  # PRECONDITION: the port must be free. A stale instance answering here would
  # silently make us measure a different process (different posture, different
  # DB) while every probe still looks healthy.
  if [ "$("$RE_CURL" -s -o /dev/null -m 2 -w '%{http_code}' "http://$RE_HOST:$port/api/version")" = "200" ]; then
    echo "[lib-re] REFUSING to boot $label: something already answers on $RE_HOST:$port" >&2
    return 1
  fi
  rm -f "$run/app.pid"
  (
    set -a
    # shellcheck disable=SC1090
    . <(sed "s/RE_PGPASS/$RE_PG_PASS/" "$run/env.sh")
    set +a
    cd "$run" || exit 1
    setsid nohup "$RE_BIN" >> "$run/app.log" 2>&1 &
    echo $! > "$run/app.pid"
  )
  local i
  for i in $(seq 1 200); do [ -s "$run/app.pid" ] && break; sleep 0.05; done
  local pid; pid=$(cat "$run/app.pid")
  for i in $(seq 1 240); do
    if grep -aq "AddrInUse" "$run/app.log" && ! kill -0 "$pid" 2>/dev/null; then
      echo "[lib-re] app($label) could not bind $port (AddrInUse); tail:" >&2
      tail -20 "$run/app.log" >&2
      return 1
    fi
    if ! kill -0 "$pid" 2>/dev/null; then
      echo "[lib-re] app($label) died during boot; tail:" >&2
      tail -20 "$run/app.log" >&2
      return 1
    fi
    if [ "$("$RE_CURL" -s -o /dev/null -m 2 -w '%{http_code}' "http://$RE_HOST:$port/api/version")" = "200" ]; then
      grep -a "Rate limiting enabled" "$run/app.log" | tail -1 \
        | sed -e 's/^[0-9T:.Z-]* *//' > "$run/startup.ratelimit.txt"
      grep -a "trusted proxy CIDRs" "$run/app.log" | tail -1 \
        | sed -e 's/^[0-9T:.Z-]* *//' > "$run/startup.trustedproxy.txt"
      return 0
    fi
    sleep 0.5
  done
  echo "[lib-re] app($label) did not answer on $port within 120s" >&2
  return 1
}

# re_shutdown <label>
re_shutdown() {
  local run="$RE_ROOT/run-$1" pid
  [ -f "$run/app.pid" ] || { echo "[lib-re] no app.pid for $1" >&2; return 0; }
  pid=$(cat "$run/app.pid")
  kill "$pid" 2>/dev/null
  local i
  for i in $(seq 1 120); do kill -0 "$pid" 2>/dev/null || { rm -f "$run/app.pid"; return 0; }; sleep 0.1; done
  kill -9 "$pid" 2>/dev/null
  sleep 0.3
  rm -f "$run/app.pid"
  # wait for the listener to disappear so the next boot cannot collide
  local p="$RE_PORT"
  if [ -n "${p:-}" ]; then
    for i in $(seq 1 60); do
      "$RE_CURL" -s -o /dev/null -m 1 "http://$RE_HOST:$p/api/version" || return 0
      sleep 0.1
    done
  fi
}

# ── log windows ──────────────────────────────────────────────────────────────
re_applog_size() { # <label>
  local f="$RE_ROOT/run-$1/app.log"
  if [ -f "$f" ]; then stat -c%s "$f"; else echo 0; fi
}
re_applog_slice() { # <label> <from> <to>
  local f="$RE_ROOT/run-$1/app.log" from="$2" to="$3" n
  [ -f "$f" ] || return 0
  n=$(( to - from ))
  [ "$n" -le 0 ] && return 0
  tail -c "+$(( from + 1 ))" "$f" | head -c "$n"
}
re_pglog_size() { docker exec -i "$RE_PG_CONTAINER" stat -c%s "$RE_PGLOG_IN" 2>/dev/null || echo 0; }
re_pglog_slice() { # <from> <to>
  local n=$(( $2 - $1 ))
  [ "$n" -le 0 ] && return 0
  docker exec -i "$RE_PG_CONTAINER" sh -c "tail -c +$(( $1 + 1 )) $RE_PGLOG_IN | head -c $n"
}

# ── case recorder ────────────────────────────────────────────────────────────
#   re_case_begin <case_id> <note>
#   re_case_step  <name> <method> <path> <ct|ABSENT> <body-file|-> [extra curl args…]
#   re_case_burst <n> <tag>  <method> <path> <ct|ABSENT> <body-file|-> [extra…]
#   re_case_reading <key> <value>
#   re_case_end
re_case_begin() {
  RE_CASE_ID="$1"; RE_NOTE="${2:-}"
  RE_CASE_DIR="$RE_OUT/cases/$RE_CASE_ID"
  rm -rf "$RE_CASE_DIR"; mkdir -p "$RE_CASE_DIR"
  RE_STEP_N=0
  RE_STATE_BEFORE=$(re_state "$RE_DB")
  RE_DB_NOW=$(re_db_now "$RE_DB")
  RE_PG_CASE_FROM=$(re_pglog_size)
  : > "$RE_CASE_DIR/readings.tsv"
}

# one curl + its meta and log windows. <extra args…> after the 6 fixed ones.
re_curl_once() { # <name> <method> <path> <ct> <body> <base> [extra…]
  local name="$1" method="$2" path="$3" ct="$4" body="$5" base="$6"; shift 6
  local hdr="$base.headers" out="$base.body" w="$base.curlout" meta="$base.meta"
  local applog="$base.app.log" pglog="$base.pg.log"
  local app_from app_af pg_from pg_af blen=0

  app_from=$(re_applog_size "$RE_LABEL");  pg_from=$(re_pglog_size)
  local wall0=${EPOCHREALTIME/./}
  local fmt='%{http_code}\t%{content_type}\t%{time_total}\t%{time_starttransfer}\t%{size_download}'
  local -a cargs=(-s -o "$out" -D "$hdr" -w "$fmt" -X "$method")
  if [ "$ct" != "ABSENT" ]; then cargs+=(-H "Content-Type: $ct"); else cargs+=(-H "Content-Type:"); fi
  [ "$body" != "-" ] && cargs+=(--data-binary "@$body")
  cargs+=("$@")
  cargs+=("http://$RE_HOST:$RE_PORT$path")
  "$RE_CURL" "${cargs[@]}" > "$w" 2>"$base.curl.err"
  local rc=$?
  local wall1=${EPOCHREALTIME/./}

  sleep "$RE_SETTLE"
  app_af=$(re_applog_size "$RE_LABEL"); pg_af=$(re_pglog_size)
  re_applog_slice "$RE_LABEL" "$app_from" "$app_af" > "$applog"
  re_pglog_slice "$pg_from" "$pg_af" > "$pglog"

  local st ctype el ttfb sz enc cl ra rid dlen=""
  st=$(cut -f1 "$w"); ctype=$(cut -f2 "$w"); el=$(cut -f3 "$w"); ttfb=$(cut -f4 "$w"); sz=$(cut -f5 "$w")
  ctype=$(grep -ai '^content-type:' "$hdr" | tail -1 | sed -E 's/^[Cc]ontent-[Tt]ype: *//' | tr -d '\r')
  enc=$(grep -ai '^content-encoding:' "$hdr" | tail -1 | sed -E 's/^[Cc]ontent-[Ee]ncoding: *//' | tr -d '\r')
  cl=$(grep -ai '^content-length:' "$hdr" | tail -1 | sed -E 's/^[Cc]ontent-[Ll]ength: *//' | tr -d '\r')
  ra=$(grep -ai '^retry-after:' "$hdr" | tail -1 | sed -E 's/^[Rr]etry-[Aa]fter: *//' | tr -d '\r')
  rid=$(grep -ai '^x-request-id:' "$hdr" | tail -1 | sed -E 's/^[Xx]-[Rr]equest-[Ii]d: *//' | tr -d '\r')
  if [ -f "$out" ]; then blen=$(stat -c%s "$out"); fi
  if [ "$enc" = "gzip" ] && [ -f "$out" ]; then
    dlen=$(gzip -dc < "$out" 2>/dev/null | wc -c)
  fi
  local body_len=0
  [ "$body" != "-" ] && body_len=$(stat -c%s "$body")

  {
    echo "name=$name"
    echo "method=$method"
    echo "path=$path"
    echo "content_type=${ct}"
    echo "extra_args=$(printf '%s|' "$@")"
    echo "body_file=$body"
    echo "body_len=$body_len"
    echo "status=${st:-0}"
    echo "response_content_type=$ctype"
    echo "response_content_encoding=$enc"
    echo "response_content_length=$cl"
    echo "retry_after=$ra"
    echo "x_request_id=$rid"
    echo "elapsed_ms=$(awk -v t="$el" 'BEGIN{printf "%.3f", t*1000}')"
    echo "ttfb_ms=$(awk -v t="$ttfb" 'BEGIN{printf "%.3f", t*1000}')"
    echo "size_download=$sz"
    echo "body_len_transferred=$blen"
    echo "body_decompressed_len=${dlen}"
    echo "wall_epoch_us=$wall0"
    echo "wall_epoch_us_end=$wall1"
    echo "curl_rc=$rc"
  } > "$meta"
  return 0
}

re_case_step() { # <name> <method> <path> <ct|ABSENT> <body|-> [extra…]
  local name="$1" method="$2" path="$3" ct="$4" body="$5"; shift 5
  RE_STEP_N=$(( RE_STEP_N + 1 ))
  local base; base=$(printf '%s/%02d-%s' "$RE_CASE_DIR" "$RE_STEP_N" "$name")
  re_curl_once "$name" "$method" "$path" "$ct" "$body" "$base" "$@"
  echo "step=$RE_STEP_N" >> "$base.meta"
}

# N simultaneous requests released by a common barrier file.
re_case_burst() { # <n> <tag> <method> <path> <ct|ABSENT> <body|-> [extra…]
  local n="$1" tag="$2" method="$3" path="$4" ct="$5" body="$6"; shift 6
  local go="$RE_CASE_DIR/.go-$tag"
  rm -f "$go"; : > "$RE_CASE_DIR/.pids-$tag"
  local i
  for i in $(seq 1 "$n"); do
    (
      while [ ! -e "$go" ]; do sleep 0.002; done
      re_curl_once "$tag-$i" "$method" "$path" "$ct" "$body" "$RE_CASE_DIR/.burst-$tag-$i" "$@"
    ) &
    echo "$!" >> "$RE_CASE_DIR/.pids-$tag"
  done
  : > "$go"
  local p
  while read -r p; do wait "$p" 2>/dev/null; done < "$RE_CASE_DIR/.pids-$tag"
  for i in $(seq 1 "$n"); do
    RE_STEP_N=$(( RE_STEP_N + 1 ))
    local new; new=$(printf '%s/%02d-%s-%d' "$RE_CASE_DIR" "$RE_STEP_N" "$tag" "$i")
    for ext in meta body headers app.log pg.log; do
      mv "$RE_CASE_DIR/.burst-$tag-$i.$ext" "$new.$ext" 2>/dev/null || true
    done
    echo "step=$RE_STEP_N" >> "$new.meta"
    rm -f "$RE_CASE_DIR/.burst-$tag-$i.curlout" "$RE_CASE_DIR/.burst-$tag-$i.curl.err"
  done
  rm -f "$go" "$RE_CASE_DIR/.pids-$tag"
}

re_case_reading() { printf '%s\t%s\n' "$1" "$2" >> "$RE_CASE_DIR/readings.tsv"; }

re_case_end() {
  RE_STATE_AFTER=$(re_state "$RE_DB")
  RE_DB_NOW_END=$(re_db_now "$RE_DB")
  re_pglog_slice "$RE_PG_CASE_FROM" "$(re_pglog_size)" > "$RE_CASE_DIR/case.pg.log"
  python3 "$RE_EMIT" \
    "$RE_OUT" "$RE_CASE_DIR" "$RE_CASE_ID" "$RE_LABEL" "$RE_NOTE" \
    "$RE_CFG_JSON" "$RE_STATE_BEFORE" "$RE_STATE_AFTER" \
    "$RE_DB_NOW" "$RE_DB_NOW_END" \
    >> "$RE_JSONL" || { echo "[lib-re] emit failed for $RE_CASE_ID" >&2; return 1; }
  rm -f "$RE_CASE_DIR"/*.curlout "$RE_CASE_DIR"/*.curl.err "$RE_CASE_DIR"/.burst-* 2>/dev/null
  echo "[lib-re] $RE_LABEL/$RE_CASE_ID done" >&2
}

# ── emitter (python, written by re_harness_init) ─────────────────────────────
re_emit_py() {
  cat > "$RE_EMIT"
}

re_harness_init() { # <outdir> <label>
  RE_OUT="$1"; RE_LABEL="$2"
  mkdir -p "$RE_OUT/cases"
  RE_JSONL="$RE_OUT/$RE_LABEL.jsonl"
  # The label names the run, so the run OWNS the file: truncate it here. Without
  # this a second run would append to the first run's records and the artifact
  # would silently mix two harness revisions.
  : > "$RE_JSONL"
  RE_EMIT="$RE_ROOT/emit_case.py"
  re_emit_py <<'PY'
import glob, gzip, json, os, re, sys, zlib

(outdir, casedir, case_id, label, note, cfg_json, st_before, st_after,
 db_now, db_now_end) = sys.argv[1:11]

TS_APP = re.compile(r'^\d{4}-\d\d-\d\dT[\d:.]+Z\s+')
ANSI = re.compile(r'\x1b\[[0-9;]*[A-Za-z]')
# A JSONL that two builds can be diffed against must carry no run-varying text.
SPAN = re.compile(r'^.*?\}: ')                       # tracing span prefix
NOISE = re.compile(r'\s(?:latency_ms|latency|content_length|rows_affected|'
                   r'rows_returned|elapsed_secs|elapsed|size_bytes)=\S+'
                   r'(?:\s*ms\b)?')
PG_DUR = re.compile(r'duration: [\d.]+ ms\s*')
CLIENT_IP = re.compile(r'client_ip=(\S+)')
EPHEMERAL_PORT = re.compile(r':\d+$')
LIMITER_IP = re.compile(r'\bip=(\S+)')
TS_PG  = re.compile(r'^\d{4}-\d\d-\d\d \d\d:\d\d:\d\d\.\d+ UTC \[\d+\] ')
MARKERS = (
    'Setup admin request received',
    'Setup admin ',
    'Rate limit exceeded',
    'claim_released',
    'claim_release_failed',
    'setup.claim',
)
INTERESTING = ('about to run the server',)
PROBE = os.environ.get('RE_PROBE_TAG', 'RE_PROBE')
FORWARDED = ('x-forwarded-for', 'x-real-ip', 'accept-encoding')


def norm_app(l):
    return ANSI.sub('', TS_APP.sub('', l.strip()))


def norm_pg(l):
    # statement text, parameters and pid stay; the measured duration does not
    # (it is the only part that differs between two identical runs).
    return PG_DUR.sub('', ANSI.sub('', TS_PG.sub('', l.strip())))


def stable_app(l):
    """Span prefix + timing fields removed => identical across runs/builds."""
    return NOISE.sub('', SPAN.sub('', l)).strip()


def kv(path):
    d = {}
    with open(path, 'r', errors='replace') as fh:
        for line in fh:
            line = line.rstrip('\n')
            if '=' in line:
                k, v = line.split('=', 1)
                d[k] = v
    return d


def state(s):
    f = (s or '').split('|')
    while len(f) < 5:
        f.append('')
    return {
        'users': int(f[0] or 0),
        'owners': int(f[1] or 0),
        'si_rows': int(f[2] or 0),
        'si_value': f[3],
        'si_updated_by': f[4],
    }


def rel(p):
    return os.path.relpath(p, outdir)


def sha(path):
    import hashlib
    h = hashlib.sha256()
    with open(path, 'rb') as fh:
        for chunk in iter(lambda: fh.read(1 << 20), b''):
            h.update(chunk)
    return h.hexdigest()


def window_prov(path, nlines):
    """Provenance for a raw log window kept on disk instead of inline.

    The committed JSONL carries every scalar plus this pointer; the full window
    stays in the sandbox ($RE_ROOT/...) and is reachable by path. Set
    RE_FULL_LOGS=1 to inline the raw line arrays instead (10x bigger artifact,
    same information).
    """
    if os.environ.get('RE_FULL_LOGS') == '1':
        return {'inline': [norm_pg(l) for l in lines(path) if PROBE not in l]}
    return {'path': rel(path), 'lines': nlines,
            'sha256': sha(path) if os.path.exists(path) else '',
            'note': 'full window lives on the sandbox host; see docs/re/runtime-evidence.md'}


def decode_body(raw, enc):
    """(decoded_bytes|None, source_label). Identity when nothing was negotiated."""
    enc = (enc or '').strip().lower()
    if raw is None:
        return None, 'no-body'
    if not enc or enc == 'identity':
        return raw, 'identity'
    try:
        if 'gzip' in enc:
            return gzip.decompress(raw), 'gzip'
        if 'deflate' in enc:
            return zlib.decompress(raw), 'deflate'
        if enc in ('br', 'brotli'):
            import brotli  # optional; absent in a bare python image
            return brotli.decompress(raw), 'br'
        if 'zstd' in enc:
            import zstandard
            return zstandard.ZstdDecompressor().decompress(raw), 'zstd'
    except Exception as exc:  # noqa: BLE001 - the label is the measurement
        return None, 'decode-failed:%s' % type(exc).__name__
    return None, 'unsupported-coding:%s' % enc


def lines(path):
    if not os.path.exists(path):
        return []
    with open(path, 'r', errors='replace') as fh:
        return [l for l in fh if l.strip()]


steps = []
step_pg_seen = set()
for meta in sorted(glob.glob(os.path.join(casedir, '*.meta'))):
    d = kv(meta)
    base = meta[:-5]
    app_f, pg_f, body = base + '.app.log', base + '.pg.log', base + '.body'
    rid = d.get('x_request_id', '')
    raw = None
    if os.path.exists(body):
        with open(body, 'rb') as fh:
            raw = fh.read()
    decoded, dec_source = decode_body(raw, d.get('response_content_encoding', ''))
    dec_file = ''
    if decoded is not None and raw is not None and len(decoded) != len(raw):
        dec_file = base + '.decoded.body'
        with open(dec_file, 'wb') as fh:
            fh.write(decoded)
    app_all = [norm_app(l) for l in lines(app_f)]
    # sqlx lines are represented once, in app_sql_statement_summaries, so they
    # are not repeated here; everything else that belongs to this request is
    # kept as a stable projection (no span prefix, no latency fields).
    app_kept = [stable_app(l) for l in app_all
                if ((rid and rid in l) or any(m in l for m in MARKERS))
                and 'sqlx::query:' not in l]
    resolved_ip = ''
    limiter_ip = ''
    for l in app_all:
        if not resolved_ip:
            m = CLIENT_IP.search(l)
            if m:
                resolved_ip = EPHEMERAL_PORT.sub('', m.group(1))
        if not limiter_ip and 'Rate limit exceeded' in l:
            m = LIMITER_IP.search(l)
            if m:
                limiter_ip = m.group(1)
    # Request-scoped SQL: the app logs every sqlx query at debug INSIDE the
    # request span, so a line carrying both the request id and `sqlx::query:` is
    # unambiguously caused by THIS request. The PG statement window cannot make
    # that attribution (background scheduler jobs write into it too), so this is
    # the per-request DB-I/O oracle and pg_statement_log_lines is corroboration.
    sql_lines = [l for l in app_all if rid and rid in l and 'sqlx::query:' in l]
    sql_sums = [m.group(1) for m in
                (re.search(r'summary="([^"]*)"', l) for l in sql_lines) if m]
    pg_kept = [norm_pg(l) for l in lines(pg_f)
               if PROBE not in l]
    step_pg_seen.update(pg_kept)
    extras = [a for a in d.get('extra_args', '').split('|') if a and a != '-H']
    fwd = sorted({a for a in extras if a.split(':', 1)[0].strip().lower() in FORWARDED})
    dlen = d.get('body_decompressed_len', '')
    steps.append({
        'step': int(d.get('step', 0) or 0),
        'name': d.get('name', ''),
        'request': {
            'method': d.get('method', ''),
            'path': d.get('path', ''),
            'content_type': d.get('content_type', 'ABSENT'),
            'extra_args': extras,
            'forwarded_headers': fwd,
            'body_file': rel(body) if os.path.exists(body) else '',
            'body_len': int(d.get('body_len', 0) or 0),
        },
        'response': {
            'status': int(d.get('status', 0) or 0),
            'content_type': d.get('response_content_type', ''),
            'content_encoding': d.get('response_content_encoding', ''),
            'content_length': d.get('response_content_length', ''),
            'retry_after': d.get('retry_after', ''),
            'x_request_id': rid,
        },
        'body_file': rel(body) if os.path.exists(body) else '',
        # body_len            = bytes that crossed the socket (curl size_download)
        # body_file_bytes     = bytes saved on disk (equal unless curl decoded)
        # body_decoded_len    = length AFTER undoing content-encoding; this is
        #                       the number to compare against a "raw 263" claim
        'body_len': int(d.get('body_len_transferred', 0) or 0),
        'body_file_bytes': len(raw) if raw is not None else 0,
        'body_decoded_len': len(decoded) if decoded is not None else None,
        'body_decoded_source': dec_source,
        'body_decoded_file': rel(dec_file) if dec_file else '',
        'body_decompressed_len': int(dlen) if dlen else None,
        'elapsed_ms': float(d.get('elapsed_ms', 0) or 0),
        'ttfb_ms': float(d.get('ttfb_ms', 0) or 0),
        'wall_epoch_us': int(d.get('wall_epoch_us', 0) or 0),
        'wall_epoch_us_end': int(d.get('wall_epoch_us_end', 0) or 0),
        'curl_rc': int(d.get('curl_rc', 0) or 0),
        # Request-id scoped: the handler logs INSIDE the request span, so a line
        # carrying both this step's request id and the marker belongs to THIS
        # request. (app_all is not usable for bursts: simultaneous requests have
        # overlapping log windows, so a window-wide match would claim every step
        # ran whenever any one of them did.)
        'handler_ran': any(rid and rid in l and 'Setup admin request received' in l
                           for l in app_all),
        'resolved_client_ip': resolved_ip,
        'limiter_ip': limiter_ip,
        'app_log_lines': app_kept,
        'app_sql_statement_count': len(sql_lines),
        'app_sql_statement_summaries': sql_sums,
        'app_log_window_lines': len(app_all),
        'app_log_file': rel(app_f),
        # Named for what it counts: LINES of the Postgres log inside this step's
        # window — NOT statements attributable to the request. Background
        # scheduler jobs write into any open window, so this number can be large
        # on a request that issued no SQL at all. The per-request oracle is
        # app_sql_statement_count / app_sql_statement_summaries.
        'pg_log_line_count': len(pg_kept),
        'pg_log_window': window_prov(pg_f, len(pg_kept)),
    })

steps.sort(key=lambda s: s['step'])

readings = {}
rf = os.path.join(casedir, 'readings.tsv')
if os.path.exists(rf):
    with open(rf, 'r') as fh:
        for line in fh:
            k, _, v = line.rstrip('\n').partition('\t')
            if k:
                readings[k] = v

# Case-level PG window = the case window MINUS every line already reported in a
# step window. What is left is other connection activity inside the case window
# (in practice the app's background scheduler jobs, which write into it
# independently of the request). Deduped so the artifact is not 40 % duplicate.
pg_case = [norm_pg(l) for l in lines(os.path.join(casedir, 'case.pg.log'))
           if PROBE not in l and norm_pg(l) not in step_pg_seen]

rec = {
    'case_id': case_id,
    'label': label,
    'note': note,
    'config': json.loads(cfg_json) if cfg_json.strip() else {},
    'db_now': db_now,
    'db_now_end': db_now_end,
    'state_before': state(st_before),
    'state_after': state(st_after),
    'pg_case_log_line_count': len(pg_case),
    'pg_case_log_window': window_prov(os.path.join(casedir, 'case.pg.log'), len(pg_case)),
    'readings': readings,
    'steps': steps,
}
print(json.dumps(rec, sort_keys=True))
PY
  chmod +x "$RE_EMIT"
}

# Env vars recorded in every JSONL record (the posture fingerprint).
# Read from the instance's own recorded env.sh so the record always describes
# the env the app was actually booted with, not whatever the caller exported.
re_cfg_json() { # [<label>]
  local f="$RE_ROOT/run-${1:-$RE_LABEL}/env.sh"
  python3 - "$f" <<'PY'
import json, sys
names = [
    'OXICLOUD_TRUST_PROXY_CIDR',
    'OXICLOUD_RATE_LIMIT_REGISTER_MAX', 'OXICLOUD_RATE_LIMIT_REGISTER_WINDOW_SECS',
    'OXICLOUD_RATE_LIMIT_LOGIN_MAX', 'OXICLOUD_RATE_LIMIT_LOGIN_WINDOW_SECS',
    'OXICLOUD_RATE_LIMIT_REFRESH_MAX', 'OXICLOUD_RATE_LIMIT_REFRESH_WINDOW_SECS',
    'OXICLOUD_RATE_LIMIT_USER_PROFILE_MAX', 'OXICLOUD_RATE_LIMIT_USER_PROFILE_WINDOW_SECS',
    'RUST_LOG',
]
env = {}
try:
    with open(sys.argv[1], 'r') as fh:
        for line in fh:
            line = line.strip()
            if line.startswith('#') or '=' not in line:
                continue
            k, v = line.split('=', 1)
            if k in names:
                env[k] = v
except FileNotFoundError:
    pass
print(json.dumps(env, sort_keys=True))
PY
}

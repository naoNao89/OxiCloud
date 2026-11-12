#!/usr/bin/env bash
# shellcheck shell=bash
# =============================================================================
# extractor-length-sweep.sh — bound the body length of POST /api/setup's axum
# JSON-extractor rejections.
#
# WHY: docs/re/final-report.md F1 rests on a 219-byte `400` observed on
# `POST /api/setup` (browser Network panel: status + size, no bytes). Two
# families can answer 400 on that route:
#
#   * the app's JSON envelope   `{"status","error","message","error_type"}`,
#     `application/json`, produced by DomainError -> AppError. For
#     `"400 Bad Request"` its length is `68 + 2*len(message) + len(error_type)`,
#     and the `email`-without-`@` branch gives `68 + 2*69 + 13 = 219`.
#   * axum's own extractor rejections, `text/plain; charset=utf-8`, produced
#     before the handler runs: `JsonSyntaxError` (400), `JsonDataError` (422),
#     `MissingJsonContentType` (415), `BytesRejection` (400/413).
#
# If the `text/plain` 400 family is bounded strictly below 219 bytes, then a
# 219-byte 400 on this route can only be the JSON envelope and F1 firms from
# "candidate" to a determination. If the family can reach 219, F1 stays a
# candidate. This script measures which.
#
# It reuses docs/re/scripts/lib-re.sh for boot/shutdown/DB/curl plumbing; the
# payloads are committed under docs/re/evidence/extractor-payloads/ and are the
# SSOT for the bytes sent (the script hashes them into the record).
#
# Sandbox-only. Defaults are a private port (18087) and DB (re_extractor) so a
# concurrent harness run on 18086 / re_case_* is never disturbed.
#
#   X_PORT=18087 X_DB=re_extractor X_LABEL=extractor \
#     bash docs/re/scripts/extractor-length-sweep.sh
#
# Outputs: $X_OUT/extractor-length-sweep.jsonl  (one record per request)
#          $X_OUT/cases/<name>/<name>.{meta,body,headers,app.log,pg.log}
# =============================================================================
set -uo pipefail

HERE=$(cd "$(dirname "$0")" && pwd)
# shellcheck source=docs/re/scripts/lib-re.sh
. "$HERE/lib-re.sh"

X_PORT=${X_PORT:-18087}
X_DB=${X_DB:-re_extractor}
X_LABEL=${X_LABEL:-extractor}
X_OUT=${X_OUT:-$RE_ROOT/out-extractor}
X_PAYLOADS=${X_PAYLOADS:-$(cd "$HERE/.." && pwd)/evidence/extractor-payloads}
X_JSONL="$X_OUT/extractor-length-sweep.jsonl"

mkdir -p "$X_OUT/cases"

# --- the case list: payload file | family | what it is meant to exercise ------
CASES=(
  "01-syntax-eof-object.json|syntax-400|the 89-byte fixture instance (EOF while parsing an object)"
  "02-syntax-control-char.json|syntax-400|longest fixed detail in the syntax alphabet (control character)"
  "03-syntax-control-char-column-2014.json|syntax-400|that longest detail at a 4-digit column"
  "04-syntax-trailing-characters.json|syntax-400|trailing characters: detail fixed, path empty"
  "05-syntax-trailing-characters-long-line.json|syntax-400|trailing characters at a 5-digit column (11 KiB body accepted)"
  "06-syntax-number-out-of-range.json|syntax-400|another fixed detail (number out of range)"
  "07-syntax-unknown-key-200.json|syntax-400|syntax error inside an UNKNOWN field value: the path echoes the 200-byte input key"
  "08-syntax-unknown-key-219-exact.json|syntax-400|same, key sized to 140 bytes: the 400 body is exactly 219 bytes"
  "09-syntax-unknown-key-4096.json|syntax-400|same, 4096-byte key: no bound above 219"
  "10-data-bare-string-219-exact.json|data-422|bare JSON string: 422 whose message echoes the payload, padded to exactly 219 bytes"
  "11-data-unknown-key-ignored.json|data-422|unknown key is IGNORED by this DTO (no deny_unknown_fields): 422 reports a missing field"
  "12-data-missing-field.json|data-422|the 99-byte fixture instance (missing field)"
  "13-data-invalid-type-integer.json|data-422|the 134-byte fixture instance (invalid type)"
  "14-syntax-control-char-in-key.json|syntax-400|the same longest detail with the error raised inside a KEY: the path is omitted (fixed-detail family)"
  "15-syntax-control-char-in-key-bigcol.json|syntax-400|fixed-detail family at a 5-digit column: still far below 219"
)

# --- boot a private instance --------------------------------------------------
# The 0.10.0 binary has NO `OXICLOUD_RATE_LIMIT_SETUP_*` knob (checked with
# `strings`); /api/setup shares the register budget, so raise THAT to keep the
# sweep off the limiter. 5/3600 s would stop the 14th request.
re_template_ensure || exit 1
re_db_reset "$X_DB"     || exit 1
re_boot "$X_LABEL" "$X_PORT" "$X_DB" \
  OXICLOUD_RATE_LIMIT_REGISTER_MAX=100000 \
  OXICLOUD_RATE_LIMIT_REGISTER_WINDOW_SECS=3600 || exit 1
trap 're_shutdown "$X_LABEL"' EXIT

echo "[sweep] instance: $(curl -s -m 3 "http://$RE_HOST:$X_PORT/api/version")"

# --- drive the sweep ----------------------------------------------------------
for entry in "${CASES[@]}"; do
  name=${entry%%|*}; rest=${entry#*|}; family=${rest%%|*}; note=${rest#*|}
  payload="$X_PAYLOADS/$name"
  if [ ! -f "$payload" ]; then
    echo "[sweep] MISSING payload $payload" >&2
    exit 1
  fi
  base="$X_OUT/cases/$name/$name"
  mkdir -p "$X_OUT/cases/$name"
  re_curl_once "$name" POST /api/setup application/json "$payload" "$base" || exit 1
  printf '%s\n' "$family" > "$base.family"
  printf '%s\n' "$note"   > "$base.note"
done

# --- emit one JSON record per request ----------------------------------------
python3 - "$X_OUT" "$X_JSONL" "$X_PAYLOADS" <<'PY'
import hashlib, json, os, sys

out, jsonl, payloads = sys.argv[1:4]
recs = []
for name in sorted(os.listdir(os.path.join(out, "cases"))):
    base = os.path.join(out, "cases", name, name)
    meta = {}
    with open(base + ".meta") as fh:
        for line in fh:
            k, _, v = line.strip().partition("=")
            meta[k] = v
    body = open(base + ".body", "rb").read()
    payload_path = os.path.join(payloads, os.path.basename(meta["body_file"]))
    payload = open(payload_path, "rb").read()
    recs.append({
        "case": name,
        "family": open(base + ".family").read().strip(),
        "note": open(base + ".note").read().strip(),
        "payload_file": "docs/re/evidence/extractor-payloads/" + os.path.basename(payload_path),
        "payload_len": len(payload),
        "payload_sha256": hashlib.sha256(payload).hexdigest(),
        "status": int(meta["status"]),
        "content_type": meta["response_content_type"],
        "content_length_header": meta.get("response_content_length", ""),
        "body_len": len(body),
        "body_sha256": hashlib.sha256(body).hexdigest(),
        "body": body.decode("utf-8", "replace"),
    })
with open(jsonl, "w") as fh:
    for r in recs:
        fh.write(json.dumps(r, sort_keys=True) + "\n")

print("%-46s %-12s %-5s %-26s %6s" % ("case", "family", "st", "content-type", "body"))
for r in recs:
    print("%-46s %-12s %-5d %-26s %6d" % (r["case"], r["family"], r["status"], r["content_type"], r["body_len"]))
    print("    %r" % (r["body"][:200],))

syntax = [r for r in recs if r["family"] == "syntax-400"]
data = [r for r in recs if r["family"] == "data-422"]
print()
print("syntax-400 (text/plain 400) max measured = %d bytes (case %s)" % (
    max(r["body_len"] for r in syntax), max(syntax, key=lambda r: r["body_len"])["case"]))
print("data-422  (text/plain 422) max measured = %d bytes (case %s)" % (
    max(r["body_len"] for r in data), max(data, key=lambda r: r["body_len"])["case"]))
print("a 219-byte 400 text/plain body was produced: %s" % any(
    r["status"] == 400 and r["content_type"].startswith("text/plain") and r["body_len"] == 219
    for r in recs))
PY
echo "[sweep] records: $X_JSONL"

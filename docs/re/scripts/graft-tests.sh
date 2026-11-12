#!/usr/bin/env bash
# =============================================================================
# graft-tests.sh — transplant ONE arm's `#[cfg(test)] mod tests` modules into a
# different source tree, so the same test bodies can be run against two builds
# (the RED side is the v0.10.0 worktree, the GREEN side is the arm itself).
#
#   usage: graft-tests.sh <arm-src-tree> <target-tree>
#
# WHY NOT A FILE COPY
#   `cp` of a whole file would drag the patch along: rate_limit.rs and
#   trusted_proxy.rs are BOTH production code and test code in one file, so a
#   whole-file copy would put the fix into the RED tree and make the experiment
#   meaningless. This script copies only the module that starts at the file's
#   `// ─── Tests ───…` banner, which every arm file places after all production
#   items, so the graft carries tests and nothing else.
#
# WHAT IS GRAFTED (per file, from the arm's OWN state — not from a merged blob)
#   src/interfaces/middleware/rate_limit.rs   append  (v0.10.0 has no tests)
#   src/interfaces/middleware/trusted_proxy.rs replace (v0.10.0 HAS a module, so
#                                                      the arm's module replaces
#                                                      it from the banner to EOF)
#   src/interfaces/errors.rs                  append  (v0.10.0 has no tests)
#   src/interfaces/api/handlers/auth_handler.rs  append, ONLY if the arm has a
#                                             test module there (0004 does); this
#                                             is the 0004 arm's spec test.
#   `append` is still idempotent-safe: if the TARGET file already carries a
#   `// ─── Tests ───…` banner the target's module is replaced instead, because
#   two `mod tests` in one file do not compile. The script prints which mode it
#   used per file, so the transcript says what happened.
#
# FAILURE HONESTY
#   A test body that references a symbol only the fix ADDS turns RED into "did
#   not compile" — which the causal gate does not accept. This script therefore
#   also greps each grafted module for identifiers that are absent from the
#   target tree's non-test code and reports them as a WARNING with the symbol
#   name; the caller must treat a compilation failure of the grafted target as
#   FAILED GATE, not RED. (The warning is a heuristic used to explain such a
#   failure; the compilation itself is the authority.)
#
# OUTPUT
#   Per file: mode used, sha256 of the arm file, of the target before and of the
#   target after, and the unified diff the graft applied. Exit 2 if any file
#   expected to carry tests in the arm does not.
# =============================================================================
set -uo pipefail

ARM=${1:?usage: graft-tests.sh <arm-src-tree> <target-tree>}
TGT=${2:?usage: graft-tests.sh <arm-src-tree> <target-tree>}
[ -d "$ARM/src" ] || { echo "no src/ in arm tree: $ARM" >&2; exit 3; }
[ -d "$TGT/src" ] || { echo "no src/ in target tree: $TGT" >&2; exit 3; }
[ "$(cd "$ARM" && pwd)" != "$(cd "$TGT" && pwd)" ] || { echo "arm and target are the same tree" >&2; exit 3; }

BANNER='^// ─── Tests ─+[[:space:]]*$'
TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT
rc=0

sha() { sha256sum "$1" | cut -d' ' -f1; }

# identifiers a grafted test module uses that the target tree does not define
# outside its test code (heuristic, for explaining a compile failure only).
undefined_symbols() { # <target-tree> <symbol-list-file>
  local tree="$1" syms="$2" s
  while read -r s; do
    [ -n "$s" ] || continue
    grep -rq --include='*.rs' -E "(fn|struct|enum|static|const|type)[[:space:]]+$s\b|\b$s[[:space:]]*:" "$tree/src" || echo "$s"
  done < "$syms"
}

graft_file() { # <relpath> <default-mode: ...> [anchor-module-name]
  local rel="$1" mode="$2" anchor="${3:-}"
  local src="$ARM/$rel" dst="$TGT/$rel"
  if [ ! -f "$src" ]; then
    if [ "$mode" = required ]; then echo "!! arm file missing: $rel" >&2; rc=2; fi
    return 0
  fi
  local modfile="$TMP/$(basename "$rel").mod"
  # Line number of the `// ─── Tests ───…` banner, if any.
  # NOTE: `grep -nF` (fixed string) and not an awk regex: this host's awk is mawk
  # in a non-UTF-8 locale and rejects the 0xE2 byte of `─` inside a dynamic
  # regex ("unexpected character 0xe2"), which silently turned every graft into
  # "no module found". Fixed-string grep matches the bytes literally.
  banner_line() { grep -nF '// ─── Tests ' "$1" | head -1 | cut -d: -f1; }
  if [ -n "$anchor" ]; then
    # Module-anchored extraction (auth_handler.rs has no ─── Tests ─── banner:
    # its 0004 module is the LAST `#[cfg(test)] mod` in the file, appended after
    # the pre-existing one). Start at the `#[cfg(test)]` immediately above
    # `mod <anchor> {` and take everything to EOF.
    local ln
    ln=$(grep -n "^mod $anchor {" "$src" | head -1 | cut -d: -f1)
    if [ -z "$ln" ]; then
      if [ "$mode" = required ]; then echo "!! module $anchor not found in $rel" >&2; rc=2; fi
      return 0
    fi
    local start=$(( ln - 1 ))
    [ "$(sed -n "${start}p" "$src")" = '#[cfg(test)]' ] || start=$ln
    sed -n "${start},\$p" "$src" > "$modfile"
  else
    local sstart
    sstart=$(banner_line "$src")
    if [ -n "$sstart" ]; then
      sed -n "${sstart},\$p" "$src" > "$modfile"
    else
      : > "$modfile"
    fi
  fi
  if ! grep -q '#\[cfg(test)\]' "$modfile"; then
    if [ "$mode" = required ]; then echo "!! no #[cfg(test)] module found in arm file: $rel" >&2; rc=2; fi
    return 0
  fi
  local had=no dstart
  dstart=$(banner_line "$dst")
  [ -n "$dstart" ] && had=yes
  local used="$mode"; [ "$had" = yes ] && used="replace"
  cp "$dst" "$TMP/before.$(basename "$rel")"
  if [ "$used" = replace ]; then
    head -n "$((dstart - 1))" "$dst" > "$TMP/head.$(basename "$rel")"
    cat "$TMP/head.$(basename "$rel")" "$modfile" > "$dst"
  else
    printf '\n' >> "$dst"
    cat "$modfile" >> "$dst"
  fi
  echo "== graft $rel  (mode=$used; target already had a module: $had)"
  echo "   arm    $src  sha256 $(sha "$src")"
  echo "   target before sha256 $(sha "$TMP/before.$(basename "$rel")")  ->  after sha256 $(sha "$dst")"
  diff -u "$TMP/before.$(basename "$rel")" "$dst" | sed -n '1,12p;/^@@/,$p' | head -80
  # heuristic symbol check: bare `Identifier::` / `Identifier(` uses in the module,
  # minus the identifiers the module itself imports (so `use tower::ServiceExt`
  # does not read as "undefined in the target tree").
  grep -oE '\b[A-Z][A-Za-z0-9_]{2,}\b' "$modfile" | sort -u \
    | grep -vE '^(TCP|JSON|SPAN|Banner|NOTE|UUID|IP|HTTP|NaN)$' > "$TMP/syms.$(basename "$rel")" || true
  grep -oE '^[[:space:]]*use[^;]*;' "$modfile" | grep -oE '\b[A-Z][A-Za-z0-9_]{2,}\b' \
    | sort -u > "$TMP/imported.$(basename "$rel")" || true
  comm -23 "$TMP/syms.$(basename "$rel")" "$TMP/imported.$(basename "$rel")" > "$TMP/need.$(basename "$rel")"
  local miss
  miss=$(undefined_symbols "$TGT" "$TMP/need.$(basename "$rel")" | tr '\n' ' ')
  [ -n "${miss// /}" ] && echo "   WARNING: identifiers not defined in $TGT/src (compile-failure risk, verify by compiling): $miss"
  return 0
}

# Which modules to graft. Default: all four. Pass a comma-separated subset as
# the third argument when an arm legitimately has no module for a file — e.g.
# the HEAD+0003 arm (C1P) carries no rate_limit.rs test module at all, because
# the module is introduced by 0001/0002 and does not exist at HEAD.
FILES=${3:-rate_limit,trusted_proxy,errors,auth_handler}
has_file() { case ",$FILES," in *",$1,"*) return 0 ;; *) return 1 ;; esac; }

has_file rate_limit    && graft_file src/interfaces/middleware/rate_limit.rs      required
has_file trusted_proxy && graft_file src/interfaces/middleware/trusted_proxy.rs   required
has_file errors        && graft_file src/interfaces/errors.rs                     required
has_file auth_handler  && graft_file src/interfaces/api/handlers/auth_handler.rs  optional setup_openapi_contract_tests

echo "== graft complete (rc=$rc)"
exit "$rc"

# RE patches — OxiCloud rate limiter, setup budget, trusted proxy, setup OpenAPI

Four standalone patch files against a clean tree at **HEAD `389e161b`** (v0.10.0
era), each with a header comment describing what it changes, why, which tests
pin it, and the expected RED/GREEN. Every patch is a `git diff` of the working
tree; apply with `git apply` from the repo root.

```sh
git apply docs/re/patches/0001-limiter-window.patch      # correctness
git apply docs/re/patches/0002-setup-budget.patch        # POLICY — optional
git apply docs/re/patches/0003-trusted-proxy-rightmost.patch
git apply docs/re/patches/0004-openapi-setup-errors.patch
```

## Patch file hashes

SHA-256 of the delivered copies — record which one an arm applied:

| Patch | sha256 (current) | superseded |
|-------|------------------|-----------|
| 0001-limiter-window.patch | `29dd53b9a5af216e0598dcdbbe0a204213eccced06117f4c108fd8e85bcb4098` | `ba230d63…` (before the configuration-count correction in the header; **checksum-identical, which is not an equivalence proof** — the tree comparison below is what establishes it); `27bac478eb5417711803798b2a47a756c06ee9f735d1d40a9c9c346c6819d4cd` (first guard-test revision, pre-reconciliation wording); `751ca5a03e53f3b1dc7e2643249f4f38fbc7d63b47aa9242dc294b178bf1e6b1` (before the guard test existed while its header advertised it); `d4e00aa9cb370119a8449ecee7b1f29869505ed8571a543d5f7e02505dfd1945` — **does not compile** (`Duration::div_ceil`, E0599; see below) |
| 0002-setup-budget.patch | `4744df0e4fc6ea2c21318f431eb155022f4e8d522fe8ca05e9db3915a3c32cc6` | `9da0aa4595901e7804cd6c7afe0ee33efb8663f8a484422f7372ef34f5e49a96`, `c1f83bad23f0d18a59014b5ee2ff8cd01976ff518c0a384f7740b0852c27bd7e`, `90e1c5e3a7696822a3ab42c3563aaa37ba31e66c3932ec303a20196715791a7a`, `83d9bcd0b213dd836831f920ad29b3266be95838b579153c62366af178317450` — all five revisions have the **same added code** (added-lines checksum identical for all, which is _not_ an equivalence proof — see below), differing only in the `rate_limit.rs` hunk's context/line numbers and in the header's 0001-pin line |
| 0003-trusted-proxy-rightmost.patch | `964e9a34be24ee25c323e33a13388c5d0ab42b5552cea98f3af05481c881aa4d` | — |
| 0004-openapi-setup-errors.patch | `e750a76928141fa16cfc0df19facade7a76f721c2cd11e3b5c522511e3dde852` | `ced64de0…` — identical added code **per the checksum, which is not an equivalence proof**; the specific edits (a `cfg(test)` comment and header wording about the now-regenerated spec) were taken from the diff itself, and `cfg(test)` text cannot reach a binary. **No arm needs re-running for it**: the edit is `cfg(test)`-level prose |

**Use 0001 and 0002 from the same row of this table.** 0002's `rate_limit.rs`
hunk is a *context* diff against 0001's test module, so a mismatched pair (an
older 0002 against this 0001, or this 0002 against an older 0001) fails at
`src/interfaces/middleware/rate_limit.rs` (~line 526) — the guard test added in
the current 0001 shifts that context. 0002's header now records the 0001 it was
generated against. Check the pair before building an arm:

```sh
git apply docs/re/patches/0001-limiter-window.patch
git apply --check docs/re/patches/0002-setup-budget.patch   # must exit 0
```

**Fixed in 0001's first copy:** `retry_after_for` rounded up with
`Duration::div_ceil(Duration::from_secs(1))`, which is **not stable std** — rustc
1.99.0 answers `E0599: no method named div_ceil found for struct
std::time::Duration`, so `HEAD+0001`, `HEAD+0001+0002` and `HEAD+all` did not
build. It is now `remaining.as_secs() + u64::from(remaining.subsec_nanos() > 0)`
(same ceil for every input; verified in the sandbox, see "Verification"), with a
code comment naming the instability so it is not "simplified" back. The hunk
header stayed `@@ -76,10 +150,44 @@` because the replacement is the same number
of lines (2 comment lines + the expression) — recomputed by `git diff`, not
edited by hand.

### Added-lines checksum (NOT an equivalence proof)

`digest()` is an **added-lines checksum**: it covers added non-comment lines
only, so it ignores deletions, target paths and `#`-prefixed lines (in Rust that
includes `#[...]` attributes). Use it to notice that a revision touched no code at
all, **never** as equivalence proof. Prose-only drift is established by diffing
the **applied trees** (or the patch files) on the same base and classifying every
differing line — `code digest equal` alone does not establish it.

```sh
digest() { awk '/^\+/ && !/^\+\+\+/ { l=substr($0,2); t=l; sub(/^[ \t]+/,"",t);
  if (t!="" && t !~ /^(\/\/|#|\*|\/\*)/) print l }' "$1" | shasum -a 256
  awk '/^\+/ && !/^\+\+\+/ { l=substr($0,2); t=l; sub(/^[ \t]+/,"",t);
  if (t!="" && t !~ /^(\/\/|#|\*|\/\*)/) print l }' "$1" | wc -l; }
digest docs/re/patches/0001-limiter-window.patch
```

| Patch | added non-comment lines | added-lines sha256 (these copies) |
|-------|------------------------|----------------------------------|
| 0001-limiter-window.patch | 249 | `41f69043aaa5c2a150d11bea7a9a14ea8a573937e630268b99218906a449a10e` |
| 0002-setup-budget.patch | 151 | `a4a7304780dbfd23f100d4b0ed319eaacee0bb03c5c448ec9962e40bef3cd854` |
| 0003-trusted-proxy-rightmost.patch | 284 | `f81b333b7edb8a700664c154306ab309563b8a9450564f6182488b3d36ac6423` |
| 0004-openapi-setup-errors.patch | 57 | `8348585d92b955ab533181f9d2a85a3dac130c8e7e1503ca39f0ef0dd0e72444` |

**What it is not.** `#`-prefixed lines are filtered out by that `awk`, and in Rust
that includes `#[test]`, `#[cfg(test)]` and `#[allow(...)]` — an attribute can
appear or vanish without moving the checksum. Deletions are invisible too (a
revision could drop a statement and still match), as are the target paths.

**What was verified instead** (sandbox `rustdev`, current 0001
`29dd53b9…`; current patch applied to a fresh `git -C /data/oxi archive HEAD`
tree, then `diff -u` with every differing line classified — comments, `#[...]`
attribute lines and code counted separately, diff metadata excluded):

| comparison | differing lines | classification |
|------------|-----------------|----------------|
| current-applied vs `c1l-new` arm source (`ac735bb0…`) | **24** | **24 comment — 0 code, 0 attributes, 0 blank**. The hunk is `@@ -477,21 +477,15 @@`, inside the guard test's `///` doc comment, i.e. inside `#[cfg(test)] mod tests` |
| current-applied vs `c1l` (`a70e6dfd…`) | 73 | 41 comment, 5 blank, 26 code + 1 `#[test]` — the guard test, added after that source was laid down; all inside `#[cfg(test)] mod tests` |
| current-applied vs `c1` (`ca71ac7d…`) | 260 | 96 comment, 16 blank, 147 code + 1 attribute — guard test + the older budget-test wording; all inside the test module |
| current-applied vs `c1p` (`d7b1e3d8…`) | 522 | that file is **HEAD's, untouched** — the proxy-only arm carries no 0001, so the diff is 0001 itself |
| **non-test source only** (everything before the `// ─── Tests ───` banner) | **0** | sha256 **`db80b2cefdb4c40e971738fb58518afafe8ca551b2d899e502857767461b70bb`**, **12 693 B** — *identical* for current, `c1l-new`, `c1l` and `c1`; `c1p`'s is HEAD's file (`d7b1e3d8…`, **6 846 B**). Sizes and hashes are taken from the same slices (`wc -c` on the extracted prefix; the `c1p` figure is the whole file, which has no test module) |

| 0001 revision | added CODE lines | added comment lines | `#` lines | added-code identical to current? |
|---------------|------------------|---------------------|-----------|----------------------------------|
| current `29dd53b9` | 249 | 214 | 6 | — |
| `27bac478` (guard revision) | 249 | 208 | 6 | **yes** — only comment counts differ |
| `46b4cfab` (build side's corrected copy) | 223 | 174 | 5 | no — 26 code lines only in current (the guard test), **0 code lines only there** |
| `751ca5a0` (post-`div_ceil`, pre-guard) | 223 | 173 | 5 | no — the same 26 (guard test) |
| `d4e00aa9` (first, non-compiling) | 223 | 173 | 5 | no — 27 only in current / 1 only there (the `Duration::div_ceil` line) |

Outcome, scoped to the file the arms diverged in: in **`rate_limit.rs`**, no
prose-edit-induced code difference exists in any pair. The only non-comment
differences there are (a) the guard test, which lives inside
`#[cfg(test)] mod tests` and therefore cannot change any binary, and (b) revisions
laid down before it existed — **provenance, not drift**: `c1l` and `c1` carry the
same production code as current (see the non-test-source row) and simply lack the
guard test, so a *current* `c1l` arm must be re-laid-down to contain it. Nothing
already built or run is invalidated by these edits.

`0001`'s **other two files** (`src/interfaces/api/handlers/grant_handler.rs`,
`src/application/services/recipient_notification_service.rs`) are checked at the
**patch** level, because the arm source trees are *partial* — `/data/re/arm-src/`
copies only the files an arm diverges in (e.g. `c1l-new` holds exactly
`src/interfaces/errors.rs` and `src/interfaces/middleware/rate_limit.rs`), so those
two paths do not exist there and a tree diff against them is meaningless (a
missing-file error reads as "differs" — worth knowing before anyone repeats the
check). At patch level they are **identical in all five published revisions**:
`grant_handler.rs` **1 347 B**, sha256 `d8772c229e7b2116...`;
`recipient_notification_service.rs` **724 B**, sha256 `7cbf262f48a7782c...`,
byte-for-byte in every copy. **"Section" is defined here so the pair is reproducible**: the patch file's
lines after that file's `diff --git` line through its last hunk line, newline-joined, **with no trailing
newline**; the size is `wc -c` of exactly that slice. (A slice that starts at `diff --git`, or keeps the
trailing newline, gives a different pair — the numbers are only useful together with this definition.)

One vocabulary note: the patch files are **frozen**, and `0004`'s header/test
comment still use the older phrase "code digest" for what this section defines as
the **added-lines checksum** (same numbers). Where that phrase appears it carries
no equivalence claim — it is prose inside a frozen patch, not a rule.

## Order and dependencies

| # | Patch | Files | Depends on |
|---|-------|-------|-----------|
| 0001 | `0001-limiter-window.patch` | `src/interfaces/middleware/rate_limit.rs`, `src/interfaces/api/handlers/grant_handler.rs`, `src/application/services/recipient_notification_service.rs` | — (applies to clean HEAD) |
| 0002 | `0002-setup-budget.patch` | `src/common/config.rs`, `src/common/config_file.rs`, `src/main.rs`, `docs/config/env.md`, **and one test hunk in `rate_limit.rs`** | **0001** (see below) |
| 0003 | `0003-trusted-proxy-rightmost.patch` | `src/interfaces/middleware/trusted_proxy.rs`, `src/interfaces/middleware/trace_span.rs` (comment), `docs/architecture/magic-link-auth.md`, `docs/config/deployment.md` | — (applies to clean HEAD) |
| 0004 | `0004-openapi-setup-errors.patch` | `src/interfaces/api/handlers/auth_handler.rs` | — (applies to clean HEAD) |

**0002 layers on 0001 in exactly one file.** `0001` leaves
`rate_limit::tests::two_routes_share_one_budget` asserting what is true of the
tree at HEAD (register and setup share one limiter, so one budget covers both);
`0002` is the patch that changes that wiring, so it owns the repoint to
`setup_and_register_have_separate_budgets`. Consequences for the differential
arms:

* `HEAD + 0001` — internally consistent: source fixes the window, the test
  asserts the (still shared) budget.
* `HEAD + 0001 + 0002` — the policy arm.
* `HEAD + 0003`, `HEAD + 0004` — each applies and runs alone.
* `HEAD + 0002` alone is **not** a valid arm: its `rate_limit.rs` hunk is a
  context diff against 0001's test module and fails to apply to a clean tree.
  That is deliberate — the alternative (putting the repoint in 0001) would make
  the `HEAD + 0001` arm carry a test asserting wiring 0001 does not change.

The combined working tree already contains all four patches applied.

## RED/GREEN per patch

Only tests whose pass/fail actually differs between the two trees appear in the
RED column. Everything else is a guard and lives in the table below.

| Patch | RED (v0.10.0 / pre-patch) | GREEN (patched) |
|-------|---------------------------|-----------------|
| 0001 | `rejected_requests_do_not_extend_the_window` fails (the request at t≈5 s is still 429 — the rejected attempt at t≈2 s slid the window to t≈6 s). `retry_after_header_shrinks_as_the_window_elapses` fails (both 429s say `Retry-After: 6`). | Both pass. |
| 0002 | No unit-RED: the limiter test builds the two instances it asserts about. The observable difference needs the HTTP layer: 5× `POST /api/auth/register` then `POST /api/setup` → **429** on the old build (register locked setup out). | Same burst → `POST /api/setup` still answers 201/400; and a setup burst leaves register's budget intact. `setup_and_register_have_separate_budgets` plus the two `common::config` tests + the existing `config_file` KNOWN scan pass. |
| 0003 | `client_ip_resolves_the_rightmost_untrusted_hop` and `spoofed_prefix_cannot_choose_the_rate_limit_key` fail (leftmost entry wins; raw text returned instead of the normalised address; spoofed prefixes select separate buckets). | Both pass; the display/client_ip byte-equality loop passes. |
| 0004 | `committed_spec_declares_the_setup_contract` was RED before the spec was regenerated — the checked-in `resources/gen/openapi.json` declared only 201/403/503 for `/api/setup` (403 and 503 did carry the `ApiErrorBody` ref; 400 and 429 were absent entirely). | **GREEN now**: the artifact was regenerated (build side, `just gen-openapi`; `docs/re/test-runs.txt`, `c1-post-regen`) and `/api/setup` declares 201 + 400/403/429/503 → `ApiErrorBody`. `setup_admin_attribute_declares_the_contract` (in-memory `ApiDoc::openapi()`) passed immediately. The spec file itself is not in any patch |

### Guards (never in a RED column)

| Test | What it holds | Why it is not evidence |
|------|---------------|------------------------|
| `concurrent_burst_admits_exactly_one_request` (0001) | `check_and_increment` stays one atomic read-decide-write per key: 64 barrier-released threads against a budget of 1 must serve exactly one, and a second key must still be served | Flaky detector of a real defect. Server level: identical on both builds (1 served / 63 rejected, 5× each — `c0-burst.tsv` / `c1-burst.tsv`; corpus cases `concurrent-burst-rejects-8` / `-16`). Unit level: overshoot fraction 0/10..8/10 depending on ambient load and arrival spread, ordering by opt-level reversing between runs — see "Atomicity guard: standalone runs". Never in a RED column |
| `two_routes_share_one_budget` (0001) → repointed to `setup_and_register_have_separate_budgets` by 0002 | One `Arc<RateLimiter>` = one budget, and 400s spend it; after 0002, a dedicated instance per route group keeps the two budgets apart | Both states are true on v0.10.0 and after the patch; it guards the accounting and the 429 shape against a refactor |

### Atomicity guard: standalone runs

`atomicity-guard-standalone-run.txt` (raw log + full tables) with the instruments
`atomicity-guard-harness.rs` (sha256 `8bfd221f…`) and
`atomicity-guard-harness-Cargo.toml` (sha256 `a7149055…`), each byte-identical to
what ran. Both `check_and_increment` bodies verbatim, in-process, 64-thread
barrier, window 15 s, moka pinned to the lockfile's `=0.12.15` with the repo's
own feature list (`["future", "sync"]`), 10 trials per configuration, run at
opt-level 0 / 1 (the repo's `[profile.dev]`, which `profile.test` inherits) / 3
(the repo's `[profile.release]`):

| opt | shape | budget | overshoot trials (run B / run A) |
|-----|-------|--------|----------------------------------|
| 0 | original | 1 | **6/10** / 3/10 |
| 1 | original | 1 | **3/10** / 1/10 |
| 3 | original | 1 | **0/10** / 6/10 |
| 0 | original | 8 | **7/10** / 8/10 |
| 1 | original | 8 | **3/10** / 1/10 |
| 3 | original | 8 | **1/10** / 3/10 |
| any | patched | 1 and 8 | 0/10 — exactly the budget, in all 6 patched configurations per run (12 across both runs), 10 trials each |

**Deterministic mode** (no threads, no timing — interleaving the two halves of
the old pair by hand, all three opt-levels identical):

```text
original  interleaved reads (A=1, B=1) -> served=2 of budget 1: A_ok=true B_ok=true
patched   same two requests, one call each -> Ok(1) then Err(()) (budget 1)
```

Reading — this is the reconciliation with the build side's three clean runs of
the repo `#[test]` on the v0.10.0 arm:

* **The non-atomicity is not stochastic.** The old shape permits a legal
  interleaving that serves 2 on a budget of 1 at every opt-level; the upsert
  shape cannot express that interleaving, and serves exactly its budget in every
  configuration measured. That is the defect and the fix.
* **Its manifestation under a barrier burst is stochastic and
  environment-dependent.** Same instrument, same opt-levels, two runs: the
  overshoot rates differ (and the ordering by opt-level reverses — opt-3 was
  6/10 in run A and 0/10 in run B). The cause of that variation is **unisolated**:
  both runs overlapped the build side's compile on 4 vCPUs — run B's load is
  recorded in `atomicity-guard-standalone-run.txt` as 2.9–5.0, while run A's
  numeric load is **not recorded** (the log says only that it, too, was under
  concurrent load). Load was never controlled, so it is a candidate alongside
  invocation parallelism and the optimisation level, not an established cause.
* Therefore the repo's test is a **flaky detector**: three clean runs on
  v0.10.0 are consistent with the 1/10–3/10 per-trial overshoot rate measured at
  the repo's own opt-1 (~0.3–0.7 probability of three clean runs), and so is a
  failure. It stays a guard, and the atomicity change is justified **by the argument
  alone** (source shape + a hand-interleaved *schedule* of that shape — not the
  production method run under controlled interleaving), never by a reproducible test.
* Next variable to control if a reproducible gate is ever wanted: **arrival
  spread** (start the threads in controlled waves, or measure on an idle machine
  with the compiler stopped) — not the optimisation level, which alone reverses
  the ordering between runs.

## Grafting a patch's tests onto a v0.10.0 worktree (per patch)

The tests are written to compile against **both** trees: they use only the
pre-existing public surface (`RateLimiter::new`, `check_and_increment`,
zero-arg `retry_after`, the `rate_limit_*` layers, `too_many_requests`,
`client_ip*`, `is_trusted_proxy`, `HeaderMap`) plus helpers defined *inside the
same test module*, which travels with them. Nothing in a test body calls a
symbol that exists only on the patched side.

Build the patch-only tree first (so 0002's arm is 0001+0002), then append that
tree's test module(s) to the v0.10.0 file. The corpus harness implements exactly
this as `docs/re/scripts/graft-tests.sh <arm-src-tree> <target-tree>` and also
warns when a grafted body references a symbol the target lacks — **that script is
authoritative for the arms**; the recipe below is the manual equivalent:

```sh
# patched = a scratch checkout with the patch(es) applied
scratch=$(mktemp -d)
git archive v0.10.0 | tar -x -C "$scratch"
( cd "$scratch" && git init -q . && git apply /abs/path/docs/re/patches/0001-limiter-window.patch )   # 0002: apply 0001 then 0002

v010=$(mktemp -d)
git archive v0.10.0 | tar -x -C "$v010"

# 0001 / 0002 → rate_limit.rs has no test module at v0.10.0, so append the whole thing:
awk '/^\/\/ ─── Tests ───────────/{p=1} p' \
  "$scratch/src/interfaces/middleware/rate_limit.rs" >> "$v010/src/interfaces/middleware/rate_limit.rs"

# 0003 → trusted_proxy.rs does have a `#[cfg(test)] mod tests` at v0.10.0:
#        replace that module with the patched one (keep the pre-existing head
#        of the file), i.e. cut from the patched file's `// ─── Tests ───`
#        marker to EOF and splice it over the v0.10.0 module.
awk '/^\/\/ ─── Tests ───────────/{p=1} p' \
  "$scratch/src/interfaces/middleware/trusted_proxy.rs" > /tmp/tp_tests.rs
# (the v0.10.0 module starts at the same marker; keep everything before it)
awk '/^\/\/ ─── Tests ───────────/{exit} {print}' "$v010/src/interfaces/middleware/trusted_proxy.rs" > /tmp/tp_head.rs
cat /tmp/tp_head.rs /tmp/tp_tests.rs > "$v010/src/interfaces/middleware/trusted_proxy.rs"

# 0004 → auth_handler.rs: append the patched new test module (it is appended
#        after the existing `registration_domain_gate_tests` module):
{ echo '#[cfg(test)]'
  sed -n '/^mod setup_openapi_contract_tests {$/,$p' \
    "$scratch/src/interfaces/api/handlers/auth_handler.rs"
} >> "$v010/src/interfaces/api/handlers/auth_handler.rs"

cd "$v010" && cargo test --lib interfaces::middleware::  # etc.
```

(The 0004 test reads `resources/gen/openapi.json` by `CARGO_MANIFEST_DIR`; no
extra setup. On v0.10.0 it fails at the 400/429 assertions, which is the point.)

Notes that keep the RED evidence honest:

* 0001's atomicity test is listed as a **guard**, not as RED evidence: the
  64-request burst against the real server serves 1 and rejects 63 on **both**
  builds (5 trials each, `c0-burst.tsv` / `c1-burst.tsv`), and the build side's
  three runs of the grafted test on the v0.10.0 arm were clean too. The standalone
  runs (previous section) explain why both outcomes are expected: the pre-patch
  shape overshoots in a *fraction* of trials that depends on ambient load and
  arrival spread (measured 0/10..8/10, ordering by opt-level reversing between
  runs), while the non-atomicity itself is a property of the source shape — so the test is a flaky
  detector, kept out of the gate, and the atomicity half of 0001 is justified by the
  argument (source shape + a schedule of it), never by a reproducible failure.
* 0002's unit test passes on both trees (it constructs the instances it asserts
  about), so its evidence is the two-arm HTTP run described above.
* 0003's tests re-exec the test binary with `OXICLOUD_TRUST_PROXY_CIDR` set —
  the CIDR list is a process-wide `OnceLock`, so they run in a child process and
  the parent asserts the child ran exactly that test and passed. The v0.10.0
  graft therefore needs no environment preparation.

## Policy flags the user must accept

1. **0002 is a policy change and is droppable.** Dropping it keeps `/api/setup`
   on the register budget (5/3600 per client IP), i.e. register traffic can
   still lock out first-run setup and a mistyped setup attempt still spends
   registration budget. Nothing in 0001/0003/0004 depends on 0002.
2. **0004's artifact step is a build action, not a patch hunk.**
   `resources/gen/openapi.json` is generated, and none of these four patches
   carries it. In the delivered working tree the build side HAS regenerated it
   (`just gen-openapi`) and `/api/setup` now declares `201` plus
   `400/403/429/503 → ApiErrorBody`, so `committed_spec_declares_the_setup_contract`
   is **GREEN** on the patched tree (`docs/re/test-runs.txt`, `c1-post-regen`).
   Anyone applying only the patches to a clean `HEAD` gets that one test RED until
   they run the same command — by design: the test guards the artifact, so an
   attribute edit without regeneration, or a regeneration that drops a status,
   fails CI instead of silently regressing every generated client.
3. **0003 changes the rate-limit key for every proxied deployment.** Operators
   behind a trusted proxy must append (`$proxy_add_x_forwarded_for`, Caddy's and
   Traefik's default) — `docs/config/deployment.md` now says so. A proxy
   configured to *replace* `X-Forwarded-For` still works (single entry = the
   client). Deployments using the deprecated trust-all flag
   (`OXICLOUD_TRUST_PROXY_HEADERS=1`) collapse to one shared bucket; documented
   in the module header.

## Regenerating the OpenAPI spec (0004) — DONE in this tree

```sh
just gen-openapi
#   = cargo run --features dev_tools --bin generate-openapi   # writes resources/gen/openapi.json
#     cd frontend && npm run gen:openapi                      # regenerates the TS SDK
```

**Status in the delivered working tree:** the build side ran it, so
`resources/gen/openapi.json` now declares `/api/setup` = `201 → PublicUserDto`
plus `400/403/429/503 → #/components/schemas/ApiErrorBody` (the 429 description
carries `Retry-After`), and `committed_spec_declares_the_setup_contract` is
GREEN. The file is **not** part of any patch — it is a generated artifact and is
listed under "Not part of any patch" below. A consumer who applies only the
patches to a clean `HEAD` must run the same command and commit the result (with
`frontend/src/lib/api/generated/openapi/`); CI's `openapi-spec-drift` job
regenerates and fails on any diff, so an attribute edit without the artifact goes
red.

## What these patches deliberately leave alone

* `docs/re/behavioral-model.md` and `docs/re/experiment-matrix.md` keep
  describing the **v0.10.0 binary's** leftmost `X-Forwarded-For` behaviour: they
  are baseline RE artifacts, and 0003 is what makes them stale on purpose.
* `src/interfaces/middleware/dpop.rs` still reads the leftmost
  `X-Forwarded-Proto/Host` — a different mechanism (it reconstructs `htu`, not a
  client identity) and a separate decision.
* The limiter's `tracing::warn!` on a setup 429 says `endpoint = "register"`:
  0002 keeps the `rate_limit_register` middleware on the setup route because a
  `rate_limit_setup` wrapper would live in 0001's file. Cosmetic; fix it in a
  patch that owns `rate_limit.rs`.
* In-process, in-memory counters only: a multi-replica deployment still
  multiplies the effective ceiling by the replica count, and `Retry-After` is
  per process. Out of scope for all four patches.
* `benches/ROUND11.md`'s note that `and_upsert_with` benchmarked slower than the
  read-then-write pair is left as the historical record it is; 0001 removes the
  code comment that repeated it, because correctness now requires the locked
  upsert.

## Not part of any patch (patch-application ≠ working tree)

Applying the four patches to a clean `HEAD` reproduces the patched **source**
byte-for-byte — verified, see below — but **not** the whole delivered working
tree. These files are in the working tree only, and an arm that needs them must
place them itself (the corpus `graft-tests.sh` already does for `errors.rs`):

| File | What it is | Why it is outside the patch set |
|------|-----------|--------------------------------|
| `src/interfaces/errors.rs` | `#[cfg(test)] mod tests` appended at EOF: the 4xx byte-length oracle for `ErrorResponse`'s wire shape and its serialisation invariants | Authored for the corpus, not by this patch series; the file has no production change, so it belongs to the harness side. Note this means an arm's `rate_limit.rs`/`trusted_proxy.rs` tests and this module are two different grafts. |
| `frontend/src/lib/api/endpoints/auth.ts` + `auth.test.ts` | SDK error-shape normalisation (body-only throws, bare-string `text/plain` bodies, `setupAdmin` message) | Frontend work from a different thread of the same investigation; no patch file, no TS type-check run by this patch author. |
| `docs/.vitepress/config.mts` | Adds `re/**` to `srcExclude` so the RE evidence bundle is skipped at site-build time | Docs-site plumbing for `docs/re/**`, not part of a code patch. |
| `docs/re/**` other than `docs/re/patches/` | RE artifacts: `behavioral-model.md`, `experiment-matrix.md`, `runtime-evidence.md`, `differential-results.md`, `final-report.md`, `evidence/`, `scripts/` (`graft-tests.sh`, `run-corpus.sh`, …) | Produced by the other agents on this investigation (baseline model, corpus harness, evidence). `patches/` is this series' output; the rest is untracked context. |
| `resources/gen/openapi.json` | The generated OpenAPI spec, +21 lines: `/api/setup` now publishes `400/403/429/503 → ApiErrorBody` | A build artifact (`just gen-openapi`), regenerated by the build side in this tree and deliberately not carried by patch 0004 — 0004 changes the `#[utoipa::path]` attribute and the test that guards the artifact, nothing else |
| `docs/re/patches/atomicity-guard-standalone-run.txt` + `atomicity-guard-harness.rs` + `atomicity-guard-harness-Cargo.toml` | Raw log and the exact instruments (harness + manifest, with the three opt-level profiles) of the unit-level atomicity runs quoted in "Atomicity guard: standalone runs" | Evidence for a claim in the patch header, not patch content: neither file is compiled by the crate (not under `src/`, no `mod` declaration), so applying the patches does not create them. |

A useful sanity check: inside an arm (clean `HEAD` + patches), `git status
--porcelain` should list exactly the files the applied patches touch, plus any
file the harness placed itself (`errors.rs`), and nothing else. In the delivered
working tree it lists the 12 patched files, the files in the table above, and
the untracked `docs/re/` + `.omp/` trees.

## Verification performed by the patch author

No repo build was run (`cargo` is blocked locally by the hook and the sandbox was
busy with the corpus agent). **The build owner must still run `cargo check` /
`cargo test`** — one class of defect (a non-stable std method) slipped through
exactly because of that, and it was found by the build agent rather than here.

* **Sandbox type-check of the ceiling expression** (this is the check that the
  earlier copy failed). Standalone file, no repo, no cargo:

  ```sh
  ibx.sh put rustdev /tmp/ceil.rs < /tmp/ceil.rs
  ibx.sh put rustdev /tmp/ceil_divceil.rs < /tmp/ceil_divceil.rs
  ibx.sh exec rustdev 'rustc --version;
    rustc --edition 2024 --crate-type lib --emit=metadata -o /tmp/ceil.rmeta /tmp/ceil.rs;
    rustc --test --edition 2024 -o /tmp/ceil_test /tmp/ceil.rs && /tmp/ceil_test;
    rustc --edition 2024 --crate-type lib --emit=metadata -o /tmp/ceil2.rmeta /tmp/ceil_divceil.rs'
  ```

  Result (`rustc 1.99.0 (b940084d7 2026-09-28)`):

  ```text
  === 1) hand-rolled ceiling, lib type-check ===
  OK: hand-rolled ceiling compiles
  === 2) same file as a test binary, values ===
  test ceil_matches_the_expected_values ... ok
  test result: ok. 1 passed; 0 failed
  === 3) Duration::div_ceil ===
  error[E0599]: no method named `div_ceil` found for struct `Duration` in the current scope
  ```

  The value test covers `0 ns → 1 s`, `999_999_999 ns → 1 s`, `1 s → 1`,
  `2 s + 1 ns → 3`, `3 s → 3`, `59.4 s → 60`, `3600 s → 3600` — the mapping
  `retry_after_for` relies on. No `.meta`/binary from that snippet is a repo
  artifact.
* **Expression byte-identity**: the three added lines in
  `docs/re/patches/0001-limiter-window.patch` (`+`-stripped) and the three lines
  in `src/interfaces/middleware/rate_limit.rs` are byte-identical (checked with
  a script, not by eye).
* `rustfmt --edition 2024 --check` clean on every edited `.rs` file.
* `git apply --check` re-run after the fix: 0001, 0003, 0004 against a clean
  `git archive HEAD` tree, and 0002 against `HEAD + 0001`.
* Applying all four to a clean `HEAD` reproduces the working tree byte-for-byte
  for all 12 patched files (`diff -q` SAME ×12), re-run after the fix.
* Every identifier referenced by the new/edited test bodies was checked against
  `git show v0.10.0:<file>`. All of them exist there: in `rate_limit.rs`
  (`RateLimiter`, `RateLimiter::new`, `check_and_increment`, zero-arg
  `retry_after`, `rate_limit_login`, `rate_limit_register`) and in
  `trusted_proxy.rs` (`client_ip_from_parts`, `client_ip_display_from_parts`,
  `is_trusted_proxy`, `parse_cidr`, `cidr_contains`). The trusted_proxy tests
  also use `RateLimiter`/`check_and_increment`, which they import from
  `rate_limit` — both exist at v0.10.0 in that file. No test body calls
  `retry_after_for` (new in 0001); it appears only in production code
  (`enforce`, `grant_handler`, `recipient_notification_service`) and in doc
  comments.
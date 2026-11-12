# Differential results — A (released) vs B (v0.10.0 source) vs C0 (HEAD) vs C1 (patched)

Method and normalization rules: `experiment-matrix.md` §6. Raw per-case records: `docs/re/evidence/` — 26 cases ×
up to 6 builds (`differential.json`, plus 36 JSONL and the per-case raw bodies/log windows referenced inside them).
Autoritative tables: `runtime-evidence.md` §B (builds, disk) and §C (RED/GREEN, verdicts, unresolved cells).

## 1. Build provenance

| Id | Tree | Source diff sha256 | Binary sha256 | Size | df `/data` free |
|---|---|---|---|---|---|
| **A** | released tag `v0.10.0` (CI, musl, `bundled-assets`, `x86-64-v2`, `--locked`) | — | `9d364fdd…e080` | 63 717 680 | downloaded, 1.7 G |
| **B** | worktree at `v0.10.0` = `496d25b8` (clean) | clean | `0f47d7a4…a179ea5` | 114 684 664 | 3.2 G → ~1 G |
| **C0** | HEAD `389e161b`, unpatched | empty | `12959e3d…422b35` | 117 662 288 | 1.4 G → 3.7 G (after `cargo clean`: the pre-existing target carried foreign flags) → 1.5 G |
| **C1L** | HEAD + `0001` only | `12f33782…` | `5f3a5b9f…f85083` | 117 743 040 | 1.7 G → 1.3 G |
| **C1P** | HEAD + `0003` only | `5fccb4a8…` | `1695ac61…1f03cb` | 117 627 856 | 928 M → 551 M |
| **C1** | HEAD + all four | `f6d9897b…` | `4666faa8…d20ec3` | 117 701 088 | 551 M → 437 M |

All local builds: dev profile, default features, `x86_64-unknown-linux-gnu`, `target-cpu=native`, `strip=debuginfo`,
`OXICLOUD_STATIC_PATH=<tree>/frontend/static` (dev builds embed no assets; identical across builds, invisible to the
API corpus). Full commands, env and per-binary hashes: `docs/re/evidence/binaries.txt`.

**Objects, precisely (this bit confused a reviewer, so it is stated once).** Seven objects are *named* in this
bundle, but only **six are builds that ran the corpus** — `A`, `B`, `C0`, `C1L`, `C1P`, `C1` — and those are exactly
`differential.json`'s labels, lower-cased as `A c0 c1l c1p c1 b`: **`b` is the `B` build**, there is no separate `B`
label. The seventh, **A′**, is the operator's live v0.10.0 instance: observed read-only (identity hash, three GETs, one
OpenAPI probe) and deliberately **never** given a corpus request, so it is an observation object, not a differential
arm. A′ is not counted in any "six builds" statement below.

**C0 is the arm that makes this differential meaningful.** Without it every B↔C1 difference would be ambiguous
between the patches and the 210 intervening commits; with it, the causal attribution rests on **C0 ↔ C1** (same
commit, same profile, only the patches differ), and C1L/C1P split the two disjoint case groups further.

## 2. The one global effect of the intervening commits

Every **domain-validation** body is exactly **one byte shorter** on the HEAD-side builds (`as_str()`
`"Invalid Input"` 13 B → `wire_name()` `"InvalidInput"` 12 B), while the handler-literal 403
(`SystemAlreadyInitialized`, 24 B) and every **extractor** body (`text/plain`, no `error_type`) are byte-identical
across all six builds. Body lengths observed:

| case | A, B | C0, C1L, C1P, C1 | delta |
|---|---|---|---|
| `setup-pw7` | 167 | 166 | −1 |
| `setup-pw8-bademail` | **219** | **218** | −1 |
| `setup-email-nodot` | 251 | 250 | −1 |
| `setup-email-forbidden` | 259 (→151 gzip) | 258 (→151 gzip) | −1 |
| `setup-username-badchars` | 317 (→167 gzip) | 316 (→166 gzip) | −1 |
| `terminal-201-then-403` (403 branch) | 224 | 224 | 0 |
| extractor 415/400/422 | 54 / 89 / 99 / 134 | 54 / 89 / 99 / 134 | 0 |

Verdict `explained-drift` for every row of that table — mechanism named, not normalized away.

## 3. Patch-targeted rows (`fix-confirmed`)

Notation: `status/bytes`, `ra=N` = the `Retry-After` header. Only the steps that differ are shown.

**RED/GREEN, measured on the current trees** (`test-runs.txt`): the grafted tests run against the clean `v0.10.0`
worktree with `rejected_requests_do_not_extend_the_window` and `retry_after_header_shrinks_as_the_window_elapses`
failing there, and the carrying arms green — `c1l` 3/3 runs (`5 passed`), `c1` 3/3 (`8 passed`). The atomicity test
inside that module is a **flaky detector**, not a RED: 9/10 failures run alone, 0/3 alongside its siblings (see §4).

| Case | A / B / C0 (original behaviour) | C1L (0001) | C1P (0003) | C1 (all) | Verdict · code path |
|---|---|---|---|---|---|
| `rate-limit-6x` | 5×`400/167`, `429/53 ra=3600` | 5×`400/166`, `429/53 **ra=3599**` | `ra=3600` | `ra=3599` | `fix-confirmed` — 0001 reports the remaining window, not the configured one |
| `retry-after-constant` | `429 ra=3600` twice (constant) | `ra=3599` then **`ra=3578`** | `3600, 3600` | `3599, 3578` | `fix-confirmed` — the value now decreases with elapsed time |
| `ttl-refresh` (register `max=1/15 s`) | `400`, then `429 ra=15` ×4 — every retry keeps the window alive | `400`, `429 ra=15`, `429 **ra=6**`, **`400`**, `429 ra=10` | unchanged from C0 | all `400` (posture effect, see §5) | `fix-confirmed` — 0001's fixed window lets the client back in one window after the *first* request; `ra` is the remaining time |
| `shared-budget` | register×5 (`201` + four `409`) → then `/api/setup` → **`429`** (shared budget) | same as C0 | same as C0 | register×5 → then `/api/setup` → **`400`** | `fix-confirmed` for 0002 — setup no longer shares the register budget (both directions) |
| `xff-honoured-trusted` | the two spoofed chains get their own buckets (leftmost hop = key); victim-shaped chain exhausts its own key | unchanged from C0 | the two spoofed chains **share one bucket**; the victim's value is still its own key | same as C1P | `fix-confirmed` for 0003 — the rightmost non-trusted hop is the key |
| `compression-bracket` | 251 raw, 259 → 151 gzip | same | same | the ≥6th setup request is `429` (posture effect, §5) | `explained-drift` — only the posture difference, see §5 |

## 4. Invariants (must be `match` on every build — a failure here blocks the gate)

| Case | Observed on A, B, C0, C1L, C1P, C1 | Verdict |
|---|---|---|
| `terminal-201-then-403` | exactly one `201/159` (role `owner`, one owner row), then `403/224` forever | `match` |
| `concurrent-valid-2` | one `201/159` + one `403/224` (order varies) | `match` (outcome class) |
| `concurrent-valid-8` (confounded: register budget 5) | exactly one `201/159` and one owner; the losers are `403/224` where the claim refuses them and `429` where the budget runs out first — **on every build, A/B/C0 included** | `match` (outcome class: one owner, one 201) |
| `concurrent-valid-{8,16}-unconfounded` (raised limiter) | exactly one `201/159`, the rest `403/224` | `match` |
| `concurrent-burst-rejects-8/16` | A/B/C0/C1L/C1P: **one** request served per burst, the other 7 (resp. 15) `429`; **C1: five of eight** served (3× `400` + 2× `403`, 3× `429`) and **five of sixteen** (2× `400` + 3× `403`, 11× `429`) — 0002 put `/api/setup` on its own budget, so the register-limited posture no longer throttles it | `match` on A/B/C0/C1L/C1P; **`explained-drift` on C1** (0002, §5.2) |
| `setup-get` | `405/0` **still spends a slot** (6 requests → `429`); identical on all builds except `ra` | `match` |
| `setup-json-*`, `setup-content-type-*` | `422/99`, `422/134`, `400/89`, `415/54`, `415/54` on every build; handler never ran (`sql=0`) | `match` |

**The one behaviour where the measurements disagree — and how it was settled.** The non-atomic counter is
invisible through HTTP (64-thread bursts served exactly 1 per trial, 5 trials on C0 and 5 on C1; the requests spread
over ≈0.4 s, `c0-burst.json` `"secs"` 0.383–0.431). Isolated in-process the instruments *disagree*: an earlier, **superseded** instrument (opt-level 0 only, different moka feature set) overshot 10/10 trials, while
the current instrument's original-shape rows overshot in **6 of 6** configurations in run A (`3,8,1,1,6,3` of 10 trials) and **5 of 6** in run B
(`6,7,3,3,0,1`) — 12 configurations per run, six of them patched — while the repository's own test, grafted onto the clean v0.10.0 tree,
passed 3/3 — with the two *other* 0001 tests failing in those same runs, so the module was live. That contradiction
is resolved by a third instrument that **models the shape**: the harness splits the old method into `read` and
`write_and_decide` (the production method is their sequential composition, `get`+1 then `insert`) and interleaves them by
hand — A reads, B reads, A writes, B writes — which serves **2 on a budget of 1 at opt-level 0, 1 and 3 identically,
while the
patched upsert answers `Ok(1)` then `Err(())` for the same two requests. So the defect is real and deterministic;
only the *burst detector* is flaky — observed overshoot rates for the original shape across runs were 0/3
(in-tree), 1–6 of 10 (the two controlled runs; run B records load 2.9–5.0, run A's figure is not in the log) and 10/10 (the superseded instrument), while the patched
shape overshot 0/10 in all six patched configurations of each run (12 across both runs). **The cause of that variation is unisolated**: load was never controlled (candidates are ambient load,
test-invocation parallelism — 9/10 alone vs 0/3 beside siblings — and optimisation level). Only my opt-level hypothesis
is falsified, on a trend reversal between two otherwise identical runs. The optimised build is where the window is narrowest, so this is a latent leniency on a tuned release server,
never observed as a limit bypass in the corpus. The detector's rate also depends on **how it is invoked**: run alone
at the repo's profile it failed **9 of 10** runs, while in the same tree alongside its four sibling tests it failed
**0 of 3** — the test runner executes the selected tests in parallel, so the 64 barrier threads share the cores and
the first insert lands before the next read: the same effect that hides the race over HTTP.

## 5. Explained drift that is *not* a patch effect, and residual cells

1. **`/api/auth/status`: `200/86` on A and B, `401/117` on C0/C1L/C1P/C1.** The intervening commits retired it
   unauthenticated; `initialized` now comes from `/api/config` (`auth_handler.rs:29`, `config_handler.rs:65`).
   `explained-drift` — and flagged upstream as an observation: **its impact on the SPA's first-run flow was not
   measured** (no frontend build/browser run in this wave; the only live mention left in the SPA is a stale
   `AUTH_PRIMITIVES` entry at `frontend/src/lib/api/transport.ts:56`).
2. **0002 changes what a register-only posture pins.** With only `OXICLOUD_RATE_LIMIT_REGISTER_*` set, C1's
   `/api/setup` falls back to its own default (5/3600), so the register tuning no longer limits it:
   `ttl-refresh` serves all five requests, `concurrent-burst-rejects-*` serve five, and the >=6th
   `compression-bracket` step is `429`. That *is* the policy decision 0002 makes; the outcome class (one 201,
   one owner) is unchanged on every build.
3. **`xff-honoured-trusted` C0→C1 residual cell** is the generic classifier reacting to the same status change
   that 0003 intends (`fix-confirmed` in the claim table); it is not a second mechanism.
4. Nothing else. Every residual `unexplained` cell in `runtime-evidence.md` §C.6 is one of the mechanisms named
   here — the ±1 wire-name drift (§2), 0001's remaining-time `Retry-After` (§3), 0002's budget split (§5.2),
   0003's rightmost-hop rule (§3), or the retired `/api/auth/status` (§5.1). The classifier reports rather than
   smooths; `diff-builds.py` reported **0 failing claims** across 26 cases.

## 6. What a reader may conclude

Every patch-targeted behaviour changed exactly as its patch intends, on the arm that carries it and not on the
arms that do not; every invariant held on all six builds; the only global difference is the one-byte wire-name
drift of the intervening commits, which is named rather than normalized; and the one behaviour the source shape
*invites* a reader to expect (a lost counter update under a concurrent burst) was **not** reproduced on any build.

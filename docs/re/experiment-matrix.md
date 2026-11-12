# Experiment matrix — `/api/setup`, rate limiter, trusted proxy, error serialization

Behavioral reverse engineering of the **shipped OxiCloud build**, not of the working tree. Rule of the
exercise: **Observe → Hypothesize → Intervene → Measure → Compare → Conclude.** Nothing here may be
concluded from source reading or from a green test run alone.

## 1. Objects under test (identity)

| Id | Object | Identity | Notes |
|---|---|---|---|
| **A** | Released binary, run in the sandbox | `oxicloud-0.10.0-x86_64-unknown-linux-musl.tar.gz` sha256 `4decd0cc…41f9`; extracted binary sha256 **`9d364fdd83f49d19dd086726b88830c2f95adc485001b16ad663dacfb665e080`** | byte-identical to the binary the operator's host runs (`/opt/oxicloud/releases/0.10.0/oxicloud`), tag `v0.10.0` |
| **A′** | The operator's live instance | same sha256, `127.0.0.1:8086` via ssh tunnel, `initialized:false`, `admin_count:0` | **GET-only.** Never POST: `/api/setup` is a one-time irreversible bootstrap |
| **B** | Source build of the same version by us | `git worktree` at tag `v0.10.0`, built in the sandbox | detects CI-vs-local build differences (features, env, flags) |
| **C** | Patched build | working tree = HEAD `389e161b` + `docs/re/patches/*` | the intervention |

Build-profile differences between A (CI release, musl, `bundled-assets`) and B/C (local, debug, no
`bundled-assets`) are **normalization items**, not findings: they may change performance and static-asset
serving, never HTTP status/message/state semantics. Any semantic difference found there must be explained by
a code path before it is reported.

## 2. Evidence grades

Every statement carries one grade; the grade never exceeds what was measured.

- `source-only` — read from code, nothing executed.
- `unit-test verified` — an executable assertion inside the repo, run green.
- `binary-observed` — produced by running A (or B/C) and capturing the bytes/state.
- `causally verified` — a controlled intervention changed exactly one factor and the predicted change appeared.
- `differentially verified` — A/B/C agree or differ in the predicted direction on the same corpus.

The operator's 219-byte body starts and stays at **candidate** until a raw body or independent branch
evidence exists — a matching length is not confirmation.

## 3. Instruments (cheapest that answers the question)

| # | Instrument | Answers |
|---|---|---|
| I1 | HTTP black-box (corpus harness, fixed order, JSONL per case) | status, headers, exact body bytes, sequencing |
| I2 | The binary's own stdout traces (`RUST_LOG=debug`) | did the handler run (`Setup admin request received …`), did the limiter reject (`Rate limit exceeded`, `endpoint`, `ip`), rollback lines (`setup.claim_released`, target `audit`) |
| I3 | Postgres `log_statement=all` | the state boundary: which SQL the app issued inside a case window (a 429 should issue none) |
| I4 | `psql` state probes before/after | `auth.users` count, `auth.admin_settings.system_initialized` (+ `updated_by`), claim row presence |
| I5 | `strace -f -e trace=network,read,write` on A | corroboration for "no DB round-trip on the 429 path"; timing-distorting, so every strace finding is re-confirmed by I1 |
| I6 | gdb / rr | only if a question is unanswerable otherwise; timing distortion recorded, black-box re-run mandatory |

No internal-variable claim may rest on a syscall trace alone.

## 4. Corpus (fixed order — the same script runs against A, B and C)

Cases and their oracles live in `docs/re/scripts/run-corpus.sh`; the ids below are the JSONL `case_id`s.

| id | input (synthetic) | oracle on the original | primary instrument |
|---|---|---|---|
| `setup-pw7` | password 7 chars, valid email | 400; `initialized` stays false; claim released | I1, I2, I4 |
| `setup-pw8-bademail` | password 8 chars, email `admin` (no `@`) | 400; **body length is the 219-candidate** | I1 (+content-type!) |
| `setup-email-nodot` | `admin@localhost` | 400 (browser-legal, server-rejected) | I1 |
| `setup-email-forbidden` | `a b@example.test` | 400; raw body > 256 B → compression probe | I1 |
| `setup-username-badchars` | `ad min` | 400; raw body > 256 B → compression probe | I1 |
| `setup-json-missing-field` | omits `email` | 422, text/plain extractor body | I1 |
| `setup-json-wrong-type` | `username: 123` | 422, text/plain | I1 |
| `setup-json-syntax-error` | truncated JSON | 400, text/plain | I1 |
| `setup-content-type-text` | valid JSON, `Content-Type: text/plain` | 415, text/plain | I1 |
| `setup-content-type-absent` | no `Content-Type` | 415 | I1 |
| `setup-get` | `GET /api/setup` | 405 **and** it spends budget (H4) | I1, I2 |
| `rate-limit-6x` | six rejections | 5 served, 6th 429 + `Retry-After: 3600`, handler did not run | I1, I2, I3 |
| `shared-budget` | exhaust via `/api/auth/register`, then `/api/setup` | 429 (both directions) | I1, I3 |
| `xff-ignored-untrusted` | two different XFF values, **no** trust list | same bucket (XFF ignored) | I1 |
| `xff-honoured-trusted` | trust list = loopback; two spoofed XFF values | different buckets (bypass); victim-shaped value burns that key | I1, I2 |
| `terminal-201-then-403` | valid full setup, then repeat | 201 (role `owner`), then 403 `SystemAlreadyInitialized` | I1, I3, I4 |
| `concurrent-valid` | N=2 / N=8 simultaneous valid POSTs, fresh DB | exactly one 201, exactly one owner (H9) | I1, I3, I4 |
| `concurrent-burst-rejects` | N=8 simultaneous rejections, `max=1`, short window | count served vs 429 (H8) | I1 |
| `ttl-refresh` | retry cadence shorter than the window, then idle ≥ window | H2 | I1 (monotonic clock) |
| `retry-after-constant` | consume, wait, request again | H3 | I1 (monotonic clock) |

Two postures, labelled on every number:

- **release posture** — no `OXICLOUD_RATE_LIMIT_*` overrides. Effective numbers are read back from the app's own
  startup line and recorded per run: `login: 10/60 s, register: 5/3600 s, refresh: 20/60 s` — i.e. register
  **5 / 3600 s** is shared by `/api/setup`. Only this posture supports claims about what the operator
  experiences.
- **shortened window** — `OXICLOUD_RATE_LIMIT_REGISTER_WINDOW_SECS=15` (second instance, second port) for the
  TTL/Retry-After experiments; the compression bracket used `register 100/3600 s`. **Never** quote a
  shortened-window number as the release posture, and never quote the repo test harness's `36000` posture as
  either.

The concurrency cases also have **unconfounded variants** added during the run, which the case list above should be
read as including: `concurrent-valid-{8,16}-unconfounded` (raised limiter, so the race is measured without the
5-per-hour budget turning losers into 429s instead of 403s) and `concurrent-burst-rejects-{8,16}` (barrier-released
bursts on `max = 1`, shortened window). For all of those, the comparable outcome is the **status multiset plus the
state counters** (`users`, `owners`, `si_rows`, `si_value`) — never which step won, never `si_updated_by` (a fresh
UUID per run), never the per-step log window. `docs/re/scripts/diff-labels.py` implements that split.

## 5. Hypotheses → intervention → falsifier

| # | Hypothesis | Intervention (one factor) | Falsifier |
|---|---|---|---|
| H1 | With a trust list configured, the limiter keys on the **leftmost** client-supplied `X-Forwarded-For` entry | `xff-honoured-trusted` vs `xff-ignored-untrusted` | spoofs collapse into one bucket ⇒ key is not client-controlled |
| H2 | A rejected request still re-inserts the key ⇒ moka resets its TTL, so retrying never unlocks and the deadline keeps moving | `ttl-refresh` + the deadline shown by `Retry-After`/behaviour across two cadences | after one window from the **first** request the client is allowed again regardless of retries |
| H3 | `Retry-After` is the configured constant, not remaining time | `retry-after-constant` | the value shrinks with elapsed time |
| H4 | The limiter middleware runs before routing/extraction, so 405s and extractor 415/422/400s spend budget | `setup-get` + `setup-json-*` then re-measure the budget | those requests do not advance the counter |
| H5 | `/api/setup` and `/api/auth/register` share one limiter instance/budget | `shared-budget` | exhausting one leaves the other unaffected |
| H6 | The observed 219-byte `/api/setup` 400 came from the `InvalidInput` email branch (v0.10.0 renders `error_type` as `as_str()` → `"Invalid Input"`, 13 B, so the message must be exactly 69 B) | reproduce every reachable 400 branch on A and compare lengths + content-type | **resolved as NOT decidable by status+length**: a 219-byte `400 text/plain` is also reachable (reproduced: `docs/re/evidence/extractor-length-bound.md`) because the extractor echoes the submitted key text, so only `Content-Type` separates the two — and it was never captured. F1 is therefore a **candidate**, not a determination |
| H7 | The setup wizard cannot display the server's message for either throw shape (object body, or the raw string a text/plain body becomes) | unit tests + the generated client's throw path (`jsonError ?? textError`) | some shape does surface the message |
| H8 | The read-then-write counter is not atomic ⇒ a simultaneous burst can serve more than `max_requests` | `concurrent-burst-rejects` (HTTP, repeated trials), an in-process burst harness, and a hand-interleaved schedule of the split halves | **source-level fact + shape-level schedule:** the pair is unsequenced, and interleaving `read`/`write_and_decide` serves 2 on a budget of 1 at every opt-level (patched: `Ok(1)`+`Err`) — a legal schedule, **not** the production method under controlled interleaving; the stochastic detector ranged 0/3–9/10 with its cause **unisolated** (load uncontrolled); over HTTP it is invisible at the ≈0.4 s spread (1/64, 5+5 trials) |
| H9 | `try_claim_initialization` is atomic (`INSERT … ON CONFLICT DO NOTHING`) ⇒ at most one owner | `concurrent-valid` at N=2 and N=8 | two owners, or two 201s |
| H10 | A validation failure releases the initialization claim (#677 behaviour) | `setup-pw7` then a valid call | the valid call answers 403 |
| H11 | Bodies **larger than 256 bytes decoded** are compressed at `CompressionLevel::Precise(4)`; ≤ 256 B are raw. (The predicate tests the decoded length, so a compressed body's *transferred* size can sit well under 256 — 259 → 151 — and a ≤ 256 transfer is not by itself evidence of a raw body.) | the two > 256 corpus cases + one ≤256 case | a ≤ 256 body arrives compressed, or a > 256 body arrives raw |
| H12 | The patched build changes only the intended behaviours; the `error_type` token drift (219 → 218) is the `as_str` → `wire_name` change from the intervening commits, not a patch effect | B vs C on `setup-pw8-bademail` + every other case | any unexplained semantic difference between A/B/C |

## 6. Differential procedure

1. Same script, same order, same inputs: `run-corpus.sh <base_url> <outdir> <label>` for A, B, C.
2. **Normalize only:** timestamps/Date/Server headers, `x-request-id`, DPoP nonce, `Content-Length` where
   compression legitimately differs by profile, absolute paths in logs.
3. **Never normalize:** status codes, response message text, content type, state transitions
   (`initialized`, claim row, user count, role), quota decisions, and the `error_type` token (a difference
   there must be explained by code path, not smoothed away).
4. Every difference gets a row in the verdict table with a code-path explanation:

| Case | A (released) | B (v0.10.0 build) | C (patched) | Expected (spec) | Verdict | Explanation |
|---|---|---|---|---|---|---|

Verdict vocabulary: `match` / `explained-drift` (same behaviour, different provenance) / `unexplained` /
`regression` / `fix-confirmed`.

## 7. Causal gate per patch

A patch is accepted only when **all** hold, each with its evidence pointer:

1. **Original failure observed** — the new test is RED against A or B, and the HTTP/state symptom is captured
   (binary-observed), not asserted.
2. **Mechanism identified** — one hypothesis above is confirmed by a controlled intervention.
3. **Patched behaviour correct** — C shows the intended status/headers/state on the same corpus row.
4. **No invariant broken** — single-owner (H9), claim release (H10), unaffected routes (`/api/auth/*`,
   `/status.php`, `/api/config` sanity), and the other corpus rows unchanged except where the patch says so.
5. **Reproducible regression test** — the test that was RED on the original is GREEN on C and lives with the
   patch.

## 8. Safety and reproducibility

- No POST/PUT/PATCH anywhere near the operator's instance (A′ is GET-only). No fuzzing, no resetting, no
  config changes on production.
- Sandbox instance binds a non-8086 loopback port with its own database, storage and synthetic secrets.
- No passwords, DPoP proofs, session tokens, DSNs or customer data in any artifact; synthetic `RE_*` values only.
- No spoofed IP/header traffic outside the sandbox.
- Do not delete build artifacts outside this work's own directories (`/data/oxi*`, `/data/re*`, `/tmp`).
- Re-run commands live in `docs/re/scripts/` and are named in each evidence section; the sandbox raw traces are
  referenced by path, the small JSONL records live in `docs/re/evidence/`.

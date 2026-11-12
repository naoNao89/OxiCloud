# Final report — OxiCloud `/api/setup`, rate limiter, trusted proxy, error serialization

Scope: the **shipped** OxiCloud build (release `v0.10.0`, binary sha256
`9d364fdd83f49d19dd086726b88830c2f95adc485001b16ad663dacfb665e080`) as it behaves when run, plus a patched
build whose differences are attributed per patch. Method: `experiment-matrix.md`. Path model:
`behavioral-model.md`. Numbers: `runtime-evidence.md`. A/B/C comparison: `differential-results.md`.

## 1. Findings

Grades are the maximum evidence actually obtained (`source-only` < `unit-test verified` <
`binary-observed` < `causally verified` < `differentially verified`); nothing is upgraded because a number
merely matched.

| # | Finding | Grade | Evidence |
|---|---|---|---|
| F1 | The operator's 219-byte `400` is consistent with the `email`-without-`@` branch — reproduced as **219 bytes, `application/json`, identity-encoded**, body verbatim `{"status":"400 Bad Request","error":"Error creating admin user: Validation error: Invalid email: missing @",…,"error_type":"Invalid Input"}` = `68 + 2·69 + 13` — **but that branch is not determined by status + length**: a 219-byte `400 text/plain; charset=utf-8` from the same endpoint is also reachable and was reproduced (`{"` + 140×`A` + `": @@@}` → `Failed to parse the request body as JSON: AAA…: expected value at line 1 column 146`). Root cause: the axum rejection echoes the **input key text** (`serde_path_to_error`'s path segment + `axum-core`'s message), and `SetupAdminDto` has no `deny_unknown_fields`, so an unknown key's value is parsed and a syntax error inside it reflects the key — growth is linear (measured 4176 bytes with a 4096-byte key; the family is unbounded up to the 10 GiB body cap). Within the domain-validation family the other lengths are 167/171/251/253/259/273/317 and none compresses to 219. | **binary-observed** (the branch, on A) **+ differentially verified** (the ±1 fingerprint across six builds); **status: candidate — most likely**, with one premise that narrows it hard but does not close the class. Only the capture's `Content-Type` (never recorded) separates the two. The premise that narrows it hardest: a request produced by the **SPA form** cannot reach the extractor — it posts `JSON.stringify({username:'admin', email, password})` (valid JSON, all three keys present, all values strings) and unknown fields are ignored — so an SPA-originated 400 must come from the **domain-validation** path, whose only 219-byte member is the `email`-without-`@` branch. That rests on reading the handler/service and the router (no other JSON-`400` emitter was found on this route); it is **not** an exhaustive proof that none exists, so the honest register stays *most likely*, and the case remains open for a hand-made (curl/devtools) request. | §A.8, §A.10, §A.22; `docs/re/evidence/extractor-length-bound.md` + `extractor-length-sweep.jsonl` |
| F2 | The SPA could not display that message: the generated client throws the parsed body, or — for a text/plain rejection — a bare **string**, while the shipped `setupAdmin`/`apiErrorFromSdkError` read an `{response, data}` envelope ⇒ `setup failed: unknown`; the `ApiError` path additionally mislabelled a server 4xx as `NetworkError`. All three shapes are now handled and pinned. | unit-test verified (RED→GREEN per shape) + the shipped asset's code path | `docs/re/evidence/frontend-vitest.txt` (RED: 3 failed / 24 passed on HEAD's file; GREEN: 27/27; tree restored, md5 verified), §A.8 for the shapes that reach it |
| F3 | The limiter keys on the **leftmost, client-supplied** `X-Forwarded-For` hop when the peer is inside the trusted CIDR: `6.6.6.6, 10.0.0.5` → bucket `6.6.6.6`, and an `XFF: 127.0.0.1` client shares the bare-peer bucket. With a trust list configured a client therefore picks its own bucket (bypass) or spends a victim's (targeted lockout). | **causally verified** — A/B/C0 key on the leftmost hop; C1P/C1 move to the rightmost non-trusted hop and the two spoofed chains collapse into one shared bucket | §A.5; `differential-results.md` §3 |
| F4 | A rejected request still re-arms the window (moka TTL reset on every write), so retrying never lets the client back in, and `Retry-After` is the **configured constant** rather than the remaining time. | **causally verified** — binary-observed on A (refused at +21.94 s past a 15 s window; +27.23 s after a re-arm at +9.65 s; idle control expires at +17.33 s; `Retry-After: 3600` at +22.01 s), and 0001's arm fixes the window (`400` served one window after the *first* request; `ra` = remaining time, `3599 → 3578`) with both tests RED on v0.10.0 | §A.2, §A.3 |
| F5 | `/api/setup` and `/api/auth/register` share one limiter instance (5/3600 s per IP by default), and the limiter runs before routing/extraction — so a `405` GET and every 415/422/400 rejection spends the same budget. | **causally verified** — binary-observed (a `GET /api/setup` consumed one of five slots; 5 × 400 then 429, `handler_ran=false`, `sql=0`); C1 shows setup surviving an exhausted *register* budget, which is 0002's change | §A.16, §A.17, §A.18 |
| F6 | The counter's read-then-write is **non-atomic by construction** — `cache.get(ip).unwrap_or(0) + 1` then `cache.insert(ip.to_string(), count)`, with nothing serialising the pair (`git show v0.10.0:src/interfaces/middleware/rate_limit.rs`). A harness that splits that method into `read` / `write_and_decide` and interleaves them by hand (A reads, B reads, A writes, B writes) serves **2 requests on a budget of 1**, identically at opt-level 0/1/3, while the patched upsert answers `Ok(1)` then `Err(())` for the same two requests — a **legal schedule of that computation**, though **not** the production method run under controlled interleaving. Its *manifestation* is stochastic and its cause is **unisolated**: the 64-thread burst detector overshot at rates from 0/3 (alongside sibling tests) to 9/10 (run alone at the repo's profile), while the patched shape overshot 0/10 in all six patched configurations of each run (12 across both runs); load was never controlled, and the opt-level hypothesis is falsified (the trend reversed between two otherwise identical runs). Through HTTP it is invisible at the spread measured — 64 requests over ≈0.4 s, 1/64 served on both builds, 5+5 trials. | **established by construction (source) + a deterministic schedule of the shape; the burst test is a flaky detector and is never counted as RED; no causal proof through the production method** | `docs/re/patches/atomicity-guard-standalone-run.txt` + `atomicity-guard-harness.rs`/`-Cargo.toml`; §A.4 |
| F7 | The initialization claim is atomic (`INSERT … ON CONFLICT DO NOTHING`) and a validation failure deletes it (row alive ≈ 4 ms, `si_rows=0` after, audit `setup.claim_released`) — so the #677 lock-out is fixed in the shipped build, and concurrent valid setup yields exactly one owner with role `owner`. | binary-observed on A | §A.6, §A.20, §A.21 |
| F8 | The compression predicate tests the **decoded** body size (`SizeAbove::new(256)`), not the transferred size: bodies of 54/89/167/**219**/251 bytes ship identity-encoded (transferred = decoded) while 259 → 151 and 317 → 167 under `Accept-Encoding: gzip`. So a transferred size in that range does **not** imply an uncompressed body — the reproduced 219-byte response is raw *because it was measured identity-encoded*, not because 219 ≤ 256. No member of the enumerable **domain-validation** family (129–167 compressed) compresses to 219; the unbounded extractor family is a separate matter — see F1/F10. | binary-observed | §A.22 |
| F9 | `/api/setup` declared no `400` and no `429` (v0.10.0's own spec has no body schema on 403/503 either; HEAD added those two refs but still lacks 400/429), so the generated client had no typed error body for the paths that matter. | **binary-observed (A′) + unit-test verified** — the v0.10.0 spec as served by the operator's instance (`docs/re/evidence/A-openapi-setup-path.json`, unauthenticated GET, 396 302 bytes: `/api/setup` responses `201/403/503`, `403`/`503` carrying no content schema) plus HEAD's committed spec; the artifact now regenerates `/api/setup` responses `[201,403,503] → [201,400,403,429,503]` and the artifact test flips RED→GREEN on C1 (`test-runs.txt`) | §A; `docs/re/patches/0004-*.patch` |
| F10 | **Informational, and the reason F1 cannot be narrowed further:** an axum extractor rejection reflects the submitted JSON **key text** into its `text/plain` body (`serde_path_to_error` path + `axum-core`'s message), so the error body is attacker-controlled and grows linearly with the payload (measured 4176 bytes from a 4096-byte key, bounded only by the route's 10 GiB body cap). No amplification or injection is claimed — the body is `text/plain`, the echo is ~1:1 with what was sent, and the SPA renders it as text — but it is the reason a `400` cannot be identified by length alone. | binary-observed | `docs/re/evidence/extractor-length-bound.md` (source quotes + 23-message table + payloads) |

**Reviewer-supplied evidence, recorded with its provenance.** A reviewer reported the historical `400`'s headers as
`content-type: application/json`, `content-length: 219`. That capture is **not in this bundle and was not observed by the
agent** — and it is **one more discriminator, not a determination**: a `application/json` `Content-Type` excludes the
`text/plain` extractor counterexample, and the enumerated **domain-validation** family contains exactly one 219-byte member,
which makes the missing-`@` branch the *most likely* cause. It does **not** establish that no other JSON-emitting `400` path
exists on this route — that exhaustiveness is not proved here — so F1 is not upgraded to "determined" by this capture. The
reviewer's own sentence is the right register: *"Historical response: application/json, 219 bytes, independently reproduced
by the missing-@ validation branch on the released binary. Most likely cause: invalid email. Exact historical response
message remains uncaptured."* The capture itself remains outside the evidence set.

## 2. Verified fixes (deliverable 6)

Per-patch detail, file lists and the RED/GREEN expectation live in `docs/re/patches/README.md`; the acceptance
gate each one had to clear is `experiment-matrix.md` §7 — with one honest carve-out: **`0002` cannot satisfy §7 items 1
and 5 as written**, because its repoint test passes on both trees by construction (there is no unit RED for a budget
split), so its evidence is the C0↔C1 HTTP intervention instead. The README says so at its own line; the gate row is not
silently rounded up.

| Patch | Change | Policy flag for the maintainer |
|---|---|---|
| `0001-limiter-window` | fixed window per key (a rejected request no longer extends it), `Retry-After` = remaining time, atomic increment | — (pure bug/DoS fix). **Caveat on the atomicity half:** the shape it fixes is demonstrated **deterministically as a schedule** — the harness splits the old method into `read`/`write_and_decide` (the production method is their sequential composition) and interleaving them serves 2 on a budget of 1, with `Ok(1)`+`Err(())` in the new one; that is a legal schedule of the computation, **not** the production method run under controlled interleaving. There is also **no reproducible stochastic test**: the 64-thread burst detector's overshoot rate ranged 0/3–9/10 across invocations (cause unisolated; load never controlled), so it is never quoted as a RED. |
| `0002-setup-budget` | `/api/setup` gets its own budget instead of sharing the register one | **policy**: changes first-run setup capacity vs register traffic |
| `0003-trusted-proxy-rightmost` | client IP = rightmost address not inside the trusted CIDR list | — (security fix; changes the key for existing proxy deployments) |
| `0004-openapi-setup-errors` | `400`/`403`/`429`/`503` + `ApiErrorBody` declared, spec regenerated, guarded by a spec test | — ; the frontend SDK regeneration is a documented follow-up (`just openapi`) |

### What the gate caught (recorded on purpose)

`0001` **as first delivered did not compile**: `retry_after_for` used `Duration::div_ceil`, which is not stable std
(rustc 1.99.0: `E0599`). Root cause is a process one, not a typo: the patch author was instructed **not** to run
cargo (local cargo is blocked by the repo's `.omp` hook and the sandbox was busy with the corpus run), so nothing
type-checked the production code — the author's own checks covered `git apply`, byte-for-byte reproduction and the
test identifiers. The build side caught it before any verdict was written and rebuilt the affected arms with a
faithful stable equivalent, `remaining.as_secs() + u64::from(remaining.subsec_nanos() > 0)` (same ceil-to-seconds
for every input), recording both patch hashes in the provenance. The verification rule changed accordingly: a
blocked build must be replaced by a **standalone `rustc` type-check of the changed expression** in the sandbox,
not by nothing.

A second nuance the same incident surfaced: `cargo build --bin oxicloud` links the library **without** `cfg(test)`,
so a successful binary build does **not** verify the `#[cfg(test)] mod tests` blocks (they are a separate compile
target). Any claim of the form "the assertions type-check" is therefore only established by an actual test-target
build — `cargo test --lib …` on the patched tree **before** the graft, so that a test-code build defect is
distinguishable from the RED gate (a grafted test that cannot compile against the un-patched source).

Two related facts a reader should not have to discover: the `#[cfg(test)]` byte-length oracle in
`src/interfaces/errors.rs` belongs to **no** patch (it predates the patch set), so "apply the four patches to a
clean HEAD" does not reproduce the working tree — `patches/README.md` lists what is outside the set, and the arm
trees are built as "patches + that file" with its sha256 recorded.

Two further process notes from the same round, kept because both cost cycles:

- **A wrong defect report costs a cycle too.** The build side reported "`0002` no longer applies on top of the
  republished `0001`" after running `git apply --check` against a **clean `HEAD`** — where `0002` has always failed
  *by design* (it is a context diff against `0001`'s test module). The real check is `0002` against `HEAD+0001`, which
  exits 0 with the current pair. `0002`'s header now records the exact `0001` it was generated against plus that
  pre-flight command, and the README says "use `0001` and `0002` from the same row".
- **Hash drift by prose is not code drift.** The arm test runs were taken while `0001` was `27bac478...`; the file now
  hashes `29dd53b9...`, the difference being header/comment prose only — established by diffing the **applied trees** on
  the same base (the non-test source is byte-identical to the current patch, `db80b2ce…`), **not** by the added-lines
  checksum: that checksum covers added non-comment lines only, so it can be equal while deletions, target paths or
  `#[...]` attributes differ, and it is never an equivalence proof. The runs therefore remain valid for the *code*; both
  hashes are quoted wherever they are cited. GREEN side re-measured on the current tree: `c1l` 3/3 runs (`5 passed`),
  `c1` 3/3 (`8 passed`); RED side: the same tests grafted onto the clean v0.10.0 worktree, with the two 0001 assertions
  failing there (`test-runs.txt`).

## 3. Limitations and open questions

1. **The historical 219-byte body is not available, and F1 is not narrowed by status + length.** What is
   exhaustive: the **domain-validation family** on this endpoint for the tested configuration — decoded
   `{167, 171, 219, 251, 253, 259, 259, 273, 317}` (fixed message strings), of which only the `> 256` members
   compress at all, shipping at 151/154/153/167, and **none compresses to 219**; the reproduced 219-byte response
   was measured **identity-encoded**. What is *not* closed — and this is the decisive negative result — is axum's
   own extractor family: its bodies are `text/plain`, its length is attacker-controlled (the rejection echoes the
   submitted **key text**), and a **219-byte instance was reproduced** on the released binary
   (`docs/re/evidence/extractor-length-bound.md`: `{"` + 140×`A` + `": @@@}` → 400, `Content-Length: 219`). The
   measured fixtures 54/89/99/134 are therefore instances, not constants (54 is the constant 415 body). So the
   identification reads: **one domain-validation body fits 219, at least one extractor body also fits 219** —
   and the deciding fact is the capture's `Content-Type`, which was never recorded. F1 stays a candidate; a
   reader must not treat "219 + 400" as selecting the JSON envelope.
   **The premise that does close it**, stated so a reader can weigh it: the SPA calls
   `sdkSetupAdmin({ body: { username: 'admin', email, password } })`, i.e. `JSON.stringify` of three string values —
   valid JSON, no unknown key, nothing for the extractor to reject (unknown fields are ignored, not denied). So an
   **SPA-originated** `POST /api/setup` can only 400 through the domain-validation envelope, whose sole 219-byte
   member is the `email`-without-`@` branch. The identification therefore holds for any request made from the
   form, and is open only for a hand-made request (curl/devtools), which the operator may also have used. Two hypotheses were also refuted along the
   way: the "422 padded by an `unknown field` name" route does not exist on this DTO (`{"x":1}` → `missing field
   \`username\``), and the 400 family is not bounded (max measured 4176 bytes, growth linear to the body cap).
2. **The operator's environment is unverifiable from here:** `/etc/oxicloud/oxicloud.env` is `root:oxicloud 0640`
   and the journal needs root, so the effective rate limits, `OXICLOUD_TRUST_PROXY_CIDR` and
   `min_password_length` (as `/api/config` shows it) could not be read. F3 therefore reads "live if a trust list
   covering the peer is configured — not verifiable from here", never "not exploitable". Their instance binds
   `127.0.0.1` only, so the uninitialised claim window is local, not LAN-wide.
3. **A/B differ by construction:** A is the CI release (release, musl, `bundled-assets`, `-C target-cpu=x86-64-v2`,
   `--locked`) while B and the C arms are local dev builds (gnu, default features, native). Those deltas can only
   affect the A↔B edge and are reported as `explained-drift` with a code path, or left `unexplained` and
   blocking — never smoothed. The causal attribution rests on **C0 ↔ C1** (same commit, same profile, only the
   patches differ).
4. **Environment changed mid-run.** The sandbox is shared: two unrelated CPU-bound processes from another
   project (~2 of 4 vCPUs, `times/etimes ≈ 0.999`) and a 54 h stray `coreutils tail` were running while the
   first pass was taken; they were terminated mid-session on the operator's instruction. Load-sensitive cases
   (elapsed-time and race outcomes) are reported from the quiet pass and the loaded pass is kept beside them;
   `runtime-evidence.md` §A.0 records the `jiffies` deltas.
5. **Postgres version delta:** the sandbox runs 18.2 (this repo's compose pin) while production runs 17.11 —
   relevant only to SQL-behaviour reasoning, not to any HTTP finding.
6. **`MANIFEST` is not part of the release tarball** (members: `LICENSE`, `oxicloud`, `example.env`,
   `README-install.md`); the file on the operator's host is installer-generated. Identity is therefore pinned by
   the binary sha256 and by `--version` (`commit=496d25b89f672ffe3395042a0584f24d490ed761`), both matched.
7. **Not measured, with reasons** (from §A.23): the operator's original 219-byte response (production must not be
   POSTed — an irreversible one-shot oracle); a proven lost update (needs a wider burst or an instrument inside
   the cache); `strace`/`gdb` on the 429 path (the per-request SQL count already proves zero I/O, and tracing
   would distort the timings); the operator's runtime config (root-only file); a crash *between* claim and
   release (an unmeasured risk, not a finding).
8. Production was read-only throughout (GET only); no fuzzing, no config change, no database touch.
9. **Upstream observation, deliberately not investigated further:** `/api/auth/status` answers `200/86` bytes
   unauthenticated on v0.10.0 but `401/117` at HEAD (the intervening commits retired it — `auth_handler.rs:29`,
   `config_handler.rs:65`; the SPA now reads `initialized` from `/api/config`). Nothing in this change set touches
   it. Its **impact on the first-run setup flow is unmeasured** — no frontend build or browser run was performed —
   and an uninitialised instance is exactly the case where that endpoint mattered, so it is worth a look before
   the next release rather than being quietly absorbed here.
10. **The atomicity finding is a shape-level demonstration plus a flaky detector — not a causal regression gate.** The
   source is unambiguous (`get`+1 then `insert` with no mutual exclusion) and the harness shows a **legal schedule** of that
   computation serving 2 on a budget of 1 at every opt-level; but that harness **splits** the method into `read` /
   `write_and_decide`, so it does not execute the production method under controlled interleaving (a monolith cannot be
   interleaved from outside without modifying it). The faithful-but-stochastic instrument is the 64-thread burst test, whose
   violation rate ranged from 0/3 to 9/10 across invocations, and **the cause of that variation is unisolated**: load was never
   controlled (candidates: ambient load, test-invocation parallelism, optimisation level). All that is falsified is my
   opt-level hypothesis, on a trend reversal. Anyone turning this into a CI gate must control the arrival spread (staged
   waves) rather than rely on the burst; the deterministic harness must be described as a schedule, never as "the original
   method under controlled interleaving".

## 4. Reproduction

Six builds (A, B, C0, C1L, C1P, C1) ran the same corpus with the same harness: `docs/re/scripts/`
(`lib-re.sh`, `run-corpus.sh`, `run-rate-limit-experiments.sh`, `graft-tests.sh`, `burst-trials.sh`,
`diff-builds.py`, `extractor-length-sweep.sh`). Per-build provenance, disk trace and per-binary sha256:
`docs/re/evidence/binaries.txt`; RED/GREEN with verbatim panics: `test-runs.txt`; machine-readable comparison:
`differential.json` (26 cases, 0 failing claims); human-readable tables: `runtime-evidence.md` §B/§C and
`differential-results.md`. Patch ordering, policy flags and per-patch graft recipes: `docs/re/patches/README.md`. Raw traces
(bodies, app logs, Postgres statement logs) stay in the sandbox and are referenced by path in
`runtime-evidence.md`.

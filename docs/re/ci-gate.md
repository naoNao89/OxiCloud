# The CI gate for the behavioural RE

`.github/workflows/re-gate.yml` is the gate. This file says what it runs, what it deliberately
does **not** run, and which check name belongs to which branch, so nobody later mistakes the
scoped run for the full one.

The RE itself is `docs/re/` — `experiment-matrix.md` (§6 differential, §7 causal gate),
`runtime-evidence.md` (§C, generated), `evidence/` (the committed JSONL + `differential.json`).
The gate re-executes the case lists in `docs/re/scripts/` against a binary built from the pushed
branch; it does not re-implement any of them.

## 1. Which check runs where

Each push to one of the seven delivery branches runs **one** leg of the matrix (the others skip
every step). The check names are the ones branch protection should require:

| branch | required check | leg | scoped cases | what it executes |
|---|---|---|---|---|
| `fix/limiter-window` | `RE gate (behaviour) — fix/limiter-window` | `c1l` | `rate-limit-6x`, `ttl-refresh`, `retry-after-constant` | `run-corpus.sh` + `run-rate-limit-experiments.sh` |
| `feat/setup-budget` | `RE gate (behaviour) — feat/setup-budget` | `c1` | `shared-budget` + the approved deadline trace (`setup-deadline-on-c1`, `setup-get-spends-setup-budget`) | `run-corpus.sh` + `run-approval-cases.sh` |
| `fix/trusted-proxy-xff` | `RE gate (behaviour) — fix/trusted-proxy-xff` | `c1p` | `xff-ignored-untrusted`, `xff-honoured-trusted` | `run-corpus.sh` + `run-rate-limit-experiments.sh` |
| `docs/setup-openapi` | `RE gate (static) — docs/setup-openapi` | — | the spec artifact + the test that guards it | `cargo run --features dev_tools --bin generate-openapi`, `git diff --exit-code resources/gen/openapi.json`, `cargo test --lib -- setup_openapi_contract` |
| `fix/spa-error-shapes` | `RE gate (static) — fix/spa-error-shapes` | — | the SDK throw-shape oracle | `npm ci` + `npx vitest run --reporter=verbose src/lib/api/endpoints/auth.test.ts` |
| `test/error-byte-oracle` | `RE gate (static) — test/error-byte-oracle` | — | the 4xx wire-byte oracle | `cargo test --lib -- error_response` |
| `docs/behavioral-re` | `RE gate (static) — docs/behavioral-re` | — | the evidence bundle's own guards | the three guards in §3 |

`RE gate (required)` additionally aggregates both matrix jobs and fails when the ref is not one of
the seven (a run started on any other ref would otherwise be green with nothing executed).

Estimates per push (ubuntu-latest, cold cache; `timeout-minutes` in brackets): behavioural legs
**20–30 min** (60) — the dev-profile build is the bulk, the corpus is ~3 min per label because the
harness resets the DB and reboots the app before every case; `docs/setup-openapi` and
`test/error-byte-oracle` **12–20 min** (45) each (two cold `cargo` builds); `fix/spa-error-shapes`
**3–5 min**; `docs/behavioral-re` **2–3 min**.

The exact invocations (ports, labels and order are the delivered ones — `A` runs first because it
owns the migration template DB every later case clones):

```sh
RE_BIN=<released v0.10.0>  # first, for each leg
bash docs/re/scripts/run-corpus.sh http://127.0.0.1:18086 "$OUT" A
RE_BIN=target/debug/oxicloud
bash docs/re/scripts/run-corpus.sh http://127.0.0.1:18086 "$OUT" c1l      # or c1 / c1p
# limiter + proxy legs only (own ports 18087-18090, own JSONL per posture):
bash docs/re/scripts/run-rate-limit-experiments.sh http://127.0.0.1:18086 "$OUT" A
bash docs/re/scripts/run-rate-limit-experiments.sh http://127.0.0.1:18086 "$OUT" c1l
# setup-budget leg only (labels are the delivered apv-* ones):
bash docs/re/scripts/run-approval-cases.sh http://127.0.0.1:18086 "$OUT" apv-a
bash docs/re/scripts/run-approval-cases.sh http://127.0.0.1:18086 "$OUT" apv-c1
# the differential, then the scoped-case gate reads its JSON:
python3 docs/re/scripts/diff-builds.py "$EVID" "$ART/differential.c1l.json" A c0 c1l
```

## 2. How it can go red (five layers, all non-vacuous)

1. **Patch presence.** `git diff $RE_BASE..HEAD` must contain a path the branch is defined by
   (matrix `marker`), and `$RE_BASE` (`389e161b`) must be an ancestor. A branch pushed without its
   patch is stopped here instead of passing a comparison of two unpatched trees.
2. **The differential's claims** (`docs/re/scripts/diff-builds.py`): every per-patch prediction that
   the script states for this label must hold against this run's own record. A failed claim is
   reported with the observed values and fails the step.
3. **Verdict reproducibility**: each scoped case must be present for this build, and its `vs_A`
   verdict must equal the one `docs/re/evidence/differential.json` records for the same label.
   `regression` fails outright. (The verdict vocabulary allows `unexplained` for rows a patch is
   *supposed* to change — `classify()` does not consult `TARGETED` — so the substantive assertions
   are layer 4, not the verdict word.)
4. **The scoped case assertions** — spelled out in the `Scoped-case gate` step: e.g. `rate-limit-6x`
   = five `400` then a `429` whose `Retry-After` is the remaining time (< 3600) with
   `handler_ran=false` and `app_sql_statement_count=0`; `ttl-refresh` `t21-req4 = 400`;
   `shared-budget` `setup-after-register = 400` **and** `register-after-setup = 201`;
   `xff-honoured-trusted` `xff-7-3 = 429` keyed on the **rightmost** untrusted hop `10.0.0.5`.
5. **The artifact guards** (§3).

`feat/setup-budget` also runs the citation check on its **fresh** records: `gen-section-c.py`
(mirrored in `/tmp` with this run's `apv-*` JSONL) must exit 0, i.e. the built binary reproduces
the deadline trace the reviewer approved (`t21-req4 = 400`, `Retry-After 15 → 6 → 10`).

## 3. The three non-vacuous guards

```sh
python3 docs/re/scripts/diff-builds.py --check docs/re/evidence/differential.json   # byte identity
python3 docs/re/scripts/gen-section-c.py /tmp/section-c.gen.md                     # then: generated text
                                                                                   # must be a verbatim
                                                                                   # block of runtime-evidence.md
```

The third guard is the **negative control** for `verify_citations()` (inside `gen-section-c.py`,
which re-derives every cited status/`Retry-After`/state and exits 2 naming the ones that drifted):
a `/tmp` mirror of the committed evidence gets exactly one bit flipped — `setup-deadline-on-c1`
(c1) `t21-req4` `400 → 429`, the row that says *a 429 does not extend the deadline* — and the step
**passes only if that mirror run fails with `CITATION ASSERTION FAILED`** and writes no output. A
green citation check is therefore never reported by a checker that cannot go red.

## 4. What is **not** runnable in CI, and why

| not run | why |
|---|---|
| `cargo clippy --all-targets --all-features`, `cargo test --all-features --workspace` | `--all-features` pulls `bundled-assets` (needs `static-dist/`, i.e. a Vite build) and `plugins` (wasmtime/extism). ci.yml's `rust-clippy`/`rust-test` own that; **re-gate is not that run** and a green re-gate says nothing about the feature matrix. |
| the 64-thread burst trials (`docs/re/scripts/burst-trials.sh`) | the RE's own conclusion (patches README, "Atomicity guard: standalone runs"): the overshoot rate depends on uncontrolled ambient load and arrival spread (0/10–8/10 across runs, ordering by opt-level reversing). It is a guard, never evidence; a flaky CI gate would be worse than none. The server-level equivalent (`concurrent-burst-rejects-8/16`, `concurrent-valid-*`) does run. |
| `strace` (I5) and gdb/rr (I6) | timing-distorting, and unavailable/unprivileged on hosted runners; every claim they once supported is re-confirmed black-box (I1) in the corpus anyway. |
| the file-based Postgres statement windows (I3 as the harness reads them, `RE_PGLOG_IN`) | a service container cannot have `logging_collector=on`: postgres is PID 1, so a restart would end the container. `log_statement=all` is set via `ALTER SYSTEM` + `pg_reload_conf()` and the statements land on the container's stderr, uploaded as `logs/pg-container.log`. The per-request SQL oracle (`app_sql_statement_count`) is unaffected, and the raw PG window counters are excluded from the differential by `diff-builds.CHURN_STEP`. |
| the raw per-case log windows (`cases/**`) | the harness's own trim rule (lib-re.sh): scalars + state + per-step SQL inline, raw windows referenced by `{path, lines, sha256}`. They stay on the runner, exactly as they stay on the sandbox. |
| the operator's instance (`A′`, `127.0.0.1:8086`) | **never**. Every request in this workflow goes to `127.0.0.1` on the runner, on throwaway ports (18086–18099), with synthetic `*.example.test` bodies. `:8086` is not contacted, and no non-loopback host is mutated. |

The released baseline is downloaded, never rebuilt: `oxicloud-0.10.0-x86_64-unknown-linux-musl.tar.gz`
is checked against the release's own `SHA256SUMS` **and** the extracted binary against
`9d364fdd83f49d19dd086726b88830c2f95adc485001b16ad663dacfb665e080` (the identity of record,
`docs/re/identity.txt` §1) before a single case is sent. The branch binary is built in-job
(`cargo build --bin oxicloud`, dev profile, default features — the provenance in
`evidence/binaries.txt`, and `RUSTFLAGS` carries `target-cpu=native` explicitly because the env var
overrides the checkout's `.cargo/config.toml`) and its sha256 is recorded beside A's.

## 5. Wiring

* **Trigger plumbing for `docs/**` and `test/**`.** ci.yml's `on.push.branches` is an allow-list, so
  `docs/setup-openapi`, `docs/behavioral-re` and `test/error-byte-oracle` were invisible to CI
  entirely. The same two globs (`docs/**`, `test/**`) are added there — one identical, eight-line
  change (two globs plus the comment that says why) on each of those three branches; the four
  `fix/**`/`feat/**` branches are already covered by that file's existing globs and get no ci.yml
  edit at all. Without it those branches never get a status check; the `changes` paths-filter still
  skips every heavy ci.yml job on them.
* **The harness.** `docs/re/scripts` + `docs/re/evidence` are read from the branch itself when it
  carries them (that is `docs/behavioral-re`), and otherwise materialised from `RE_HARNESS_REF`
  (`docs/behavioral-re`) with `git fetch --depth=1` + `git checkout FETCH_HEAD -- …`. So a
  behavioural branch does not need a copy of the harness, and two arms cannot silently run two
  different harnesses. **Prerequisite:** `docs/behavioral-re` must exist on the fork with
  `docs/re/{scripts,evidence}` committed; without it every behavioural leg fails at the
  materialisation step (loudly, by design).
* **Baselines.** `A` (released, hash-pinned) and the branch label are this run's own records;
  `c0` (the unpatched build of the same commit) comes from `docs/re/evidence/`, whose staleness is
  what `--check` guards. Re-deriving `c0` per run would double the corpus time for a comparison the
  committed artifact already pins.
* **Artifacts** (`re-gate-<label>` / `re-gate-static-<kind>`, 30 days): the corpus JSONL, the
  differential JSON + summary, both binaries' hashes and the A tarball hash, the workflow run id,
  the commit SHA and the payload diff hash, the redacted app logs and effective rate-limit/trusted-
  proxy startup lines, and the redacted Postgres container log. Redaction is the harness's own
  `lib-re.sh::re_redact`.
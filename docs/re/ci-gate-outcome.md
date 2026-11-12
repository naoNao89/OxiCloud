# CI gate outcome — blocked on the fork; what the sandbox verified instead

## The blocker (confirmed by GitHub, not inferred)

- **18 runs exist on `naoNao89/OxiCloud`; every single one is `completed/failure`, and no job has ever executed a
  step.** `gh api …/actions/runs/<id>/jobs` returns `steps: 0` for each job, with
  `runner_id: 0`, `runner_name: ""`, `runner_group_name: ""`, and a duration of ~1 s.
- Reproduced with a **seven-line** workflow on a scratch branch (`.github/workflows/smoke.yml`: `on: push`, one
  `run: |` step, `permissions: contents: read`, no services, no matrix, no third-party action): run
  **43297983451**, job `hi`, `steps: 0`. The scratch branch was deleted afterwards; the run id is kept here as the
  evidence. (Two earlier smoke attempts failed for a *real* YAML bug of ours — `": "` inside a plain scalar — which
  was fixed before this run; that bug was in the smoke, never in the shipped workflows.)
- `GET /repos/naoNao89/OxiCloud/actions/permissions` → `{"enabled":true,"allowed_actions":"all"}`; the repository is
  public; `runs-on: ubuntu-latest` is a valid label; `actionlint` 1.7.12 reports **no findings** on
  `re-gate.yml` or `ci.yml`.
- **The reason, in GitHub's own words.** Each job's check run carries a failure annotation, read from
  `gh api <job.check_run_url>/annotations`:
  ```json
  {"annotation_level":"failure",
   "message":"The job was not started because your account is locked due to a billing issue."}
  ```
  The same message is the only failure annotation on **every** failing job checked — the smoke job
  (check run 131461286817), the `CI` run 43296304317, the `RE gate` run 43296304348, and the frontend branch's
  run 43298315578. (The run-level `…/actions/runs/<id>/annotations` endpoint returns 404 and is *not* the
  instrument; the annotation hangs off the **check run**, whose URL is the job's `check_run_url` field.)
- Conclusion: hosted execution is blocked by an **account billing lock** on the account owning
  `naoNao89/OxiCloud`. Nothing ran, so this evidence neither implicates nor clears the workflow files at runtime;
  their validity rests on the offline instruments only — `actionlint` 1.7.12 (no findings), `bash -n` on every
  `run:` block, and the fact that the non-vacuous guards were **executed locally** (below). Unblocking is
  a billing matter on the account, outside this repository.
- Incidental, from the same annotation payload: GitHub also attaches a `notice` that the `ubuntu-latest` label
  migrates to Ubuntu 26 from 2026-10-19 ([runner-images#14748](https://github.com/actions/runner-images/issues/14748)) —
  which is already in the past relative to this record, so the runners' image is moving underneath any future run.

## What each phase therefore is

| phase | state |
|---|---|
| 3 — configure CI | **done**: `.github/workflows/re-gate.yml` identical on all seven branches + the two `ci.yml` trigger globs on the three refs `ci.yml`'s allow-list would otherwise miss; actionlint-clean; every `run:` block `bash -n`-clean; the guards were *executed* locally (below) |
| 4 — behavioural RE as a required gate | **configured, cannot execute on this fork**; the behaviour evidence is the sandbox run (below), whose results carry the same claims the gate would assert |
| 6 — push and verify GitHub CI | **blocked**: branches pushed, run ids retrieved, every run failed at runner assignment ⇒ no GitHub artifact exists to compare, and no CI result is claimed |
| 7 — delivery matrix | below, with `CI = blocked (runner_id 0)` |

## Sandbox verification that did pass (the RE evidence)

Rust branches — gate run in the sandbox (`rustdev`, rustc 1.99.0) on trees **rebuilt from the branch + its patch**,
with every changed file's sha256 compared to the local worktree first:

| branch | `fmt --check` | `clippy --lib --tests -D warnings` | branch-filtered tests |
|---|---|---|---|
| `fix/limiter-window` | 0 | 0 | `rate_limit` → 5 passed |
| `feat/setup-budget` | 0 | 0 | 7 passed |
| `fix/trusted-proxy-xff` | 0 | 0 | 9 passed |
| `docs/setup-openapi` | 0 | 0 | 6 passed |
| `test/error-byte-oracle` | 0 | 0 | 2 passed |

Frontend branch — gate run **in the sandbox** (Node 24.22.3 + npm 11.19.1 under `/data/node24`, `npm ci` → 793
packages):

| command | result |
|---|---|
| `npm run check` | rc 0 — `svelte-check found 0 errors and 0 warnings`, Prettier clean, eslint/stylelint silent |
| `npm run test:unit` | rc 0 — `Test Files 122 passed | 1 skipped`, `Tests 882 passed | 2 skipped` |
| `npm run build` | rc 0 — `✓ built in 9.14s`, `static-dist` 11 756 684 B, 685 files, 219 `.br` + 219 `.gz` |
| RED leg (revert `auth.ts` to HEAD's copy) | rc 1 — `Tests 3 failed | 24 passed`, exactly the three throw-shape tests; restored file hash verified, GREEN control 27/27 |

Behavioural RE (the differential behind the patches) is the frozen bundle on this branch: six builds
(A = released v0.10.0 binary, B = same-version source build, C0 = unpatched HEAD, C1 = all four patches, C1L, C1P),
26 corpus cases, `diff-builds.py` 0 failing claims, and the branch-content composition check showing each branch's
production code is byte-identical to the arm tree that was measured (`db80b2ce…` for every 0001-carrying tree).

## Phase 7 matrix

| Branch | Base SHA | HEAD SHA (local == remote) | CI run | CI | RE | Artifacts |
|---|---|---|---|---|---|---|
| `docs/behavioral-re` | `389e161b` | `b3dca850` | 43296304407 / 43296304058 | **blocked (runner_id 0)** | bundle + pin `7f9c2014`; guards executed locally | bundle committed (101 files under `docs/re/`) |
| `fix/limiter-window` | `389e161b` | `e8f05116` | 43296304348 / 43296304317 | **blocked** | sandbox gate green; behaviour from the `c1l` arm | 3 payload files |
| `feat/setup-budget` | `fix/limiter-window` | `1d53c376` | 43296316206 / 43296316090 | **blocked** | sandbox gate green; deadline trace + budget split measured on C1 | 5 payload files |
| `fix/trusted-proxy-xff` | `389e161b` | `d1377923` | 43296324107 / 43296324135 | **blocked** | sandbox gate green; XFF chains collapse to one bucket on this arm | 4 payload files |
| `docs/setup-openapi` | `389e161b` | `3dcaed6e` | 43296334352 / 43296334094 | **blocked** | sandbox gate green; spec artifact test RED→GREEN | 2 payload files |
| `fix/spa-error-shapes` | `389e161b` | `55cb278d` | 43298315521 / 43298315578 | **blocked** | sandbox frontend gate green incl. the RED leg | 2 payload files |
| `test/error-byte-oracle` | `389e161b` | `dd6ff75b` | 43296343712 / 43296343641 | **blocked** | sandbox gate green | 1 payload file |

Every branch additionally carries the identical `re-gate.yml` (+ the `ci.yml` trigger globs where its name is not
already matched) in its own `ci:` commit.

## Residual uncertainties, stated

1. No CI result exists for any branch; nothing here may be read as CI-passed.
2. The frontend sandbox gate used **Node 24.22.3** (the repo's `engines` is `>=24`), while `ci.yml` pins **26.3.0** —
   a deviation in the sandbox environment, not in the artifact.
3. Clippy ran as `--lib --tests` with default features per branch; `--all-features --all-targets` (and the
   integration-test tree) are CI's job and did **not** run.
4. The docs branch's VitePress build fails on a **pre-existing** upstream dead link
   (`/plan/normalize-file-dates`, referenced from `architecture/dates.md`); no workflow builds the docs site on these
   branches, and the failure is not from this work.

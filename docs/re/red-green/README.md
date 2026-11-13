# RED/GREEN proof gate — the manifest contract

`.github/workflows/red-green-gate.yml` proves a **transition**, not a state: it checks out the recorded
test-only commit, requires the named test to FAIL there for the expected reason, checks out the fix, and
requires the *same* test file to PASS. A green suite on the tip cannot distinguish "this test went red then
green" from "this test never ran", and that is the difference this gate exists to make.

## `manifest.json`

```json
{
  "branches": [
    {
      "branch":  "fix/limiter-window",
      "kind":    "rust",
      "red":     "<sha of the test-only commit>",
      "green":   "<sha of the fix commit>",
      "test_path": "src/interfaces/middleware/rate_limit.rs",
      "filter":  "<the cargo test filter that selects the regression test(s)>",
      "expect_red":   "<text that must appear in the RED output — the assertion, not the build error>",
      "expect_green": "<text that must appear in the GREEN output — the passing line>"
    }
  ]
}
```

`kind` selects which leg runs, and what "unchanged" is compared against:

| `kind` | leg command | the frozen comparison |
|---|---|---|
| `rust` | `cargo test --lib -- <filter>` | the whole `#[cfg(test)]`-to-EOF region of `test_path` |
| `integration` | `cargo test --test <test_target> -- <filter>`, against the job's postgres service | the whole file at `test_path` (repo-relative) |
| `frontend` | `npx vitest run <test_path>` from `frontend/` | the whole file at `frontend/<test_path>` |

For `frontend`, `filter` is unused and `expect_red`/`expect_green` are substrings of Vitest's output.
For `integration`, `test_target` is the cargo target name, and three rules make a **skip** impossible to
mistake for a pass — because libtest captures a passing test's stdout and reports `1 passed` whether or not
the test did anything:
- the legs pass `--nocapture`, so the test's own output (including its `SKIP` line) reaches the log;
- each leg **fails closed** if `OXICLOUD_TEST_DB_URL` is unset, rather than letting the test skip;
- the legs fail if `SKIP` appears at all, and `expect_green` is a **behavioural** marker (the post-fix startup
  line, e.g. `setup: 2/3600 s`) rather than the test name — a skip reports the same `1 passed` summary, so the
  name would not discriminate.

**Values must be shell-safe**: they are interpolated into `run:` blocks as `${{ … }}`, so a double quote ends
the shell string early, and a backtick, `$` or backslash is interpreted. The gate checks for exactly that and
refuses the whole run with a clear error rather than failing three steps later — pick a quote-free substring
of the assertion (e.g. `must declare 400` for an assertion whose full text is
`must declare 400; it declares ["201", "403", "503"]`).

## What the gate refuses (each is an explicit error, not a silent pass)

1. a `red` or `green` SHA that is not a commit in the pushed history;
2. a `red` that is not an ancestor of `green` (a rebase that inverted the order would otherwise "prove" something);
3. a `green` that is not an ancestor of the pushed HEAD;
4. a test that the fix **touched**, in the two shapes the tests come in:
   - **frontend** — a separate file (`frontend/src/lib/api/endpoints/auth.test.ts`): whole-file byte-identity,
     on a path normalised to the repo root (the manifest is frontend-relative; an unnormalised
     `git diff --quiet src/lib/…` matches nothing at the repo root and exits 0 *vacuously* — that bug is fixed);
   - **rust** — the test module is inline in the same file the fix changes, so the unit of comparison is the
     whole region from the first `#[cfg(test)]` to EOF, and it must be byte-identical. That is what "the test
     was not weakened" means, and it is strictly stronger than checking that the lines the test commit added
     are still present somewhere: byte-identity also rejects an added `#[ignore]`, a new `#[cfg]` gate, an
     early return, or a reworded assertion, all of which leave the original lines in place;
   - in both cases the manifest's `filter` must name something inside the compared region, so a filter that
     matches nothing cannot masquerade as a frozen test;
   - and the comparison runs twice: against the **fix commit** (the causal transition) and against the
     **pushed HEAD** (what is actually delivered), because a later commit could re-touch the test or the code
     after the fix and the transition would still read green;
5. a RED run that **exits zero** (the test passed on the unpatched tree);
6. a RED run whose failure is a **build** error (`error[E…]`, `could not compile`, `cannot find`, `unresolved import`, `linking`, `failed to select a version`) — that is not evidence of the behavioural defect;
7. a RED run with no `test result: FAILED` / `failures:` marker, or with `running 0 tests` (filtered to nothing);
8. a RED run whose output lacks `expect_red`;
9. a GREEN run that does not report `test result: ok. N passed` (N ≥ 1) or lacks `expect_green`.

## What this gate does NOT prove — its residual holes, stated rather than implied

An independent auditor attacked the inline-test check and found cases it does not cover. They are listed here
because a gate whose limits are unknown is worse than one whose limits are documented:

- **The inline-test region is a heuristic, not a proof.** It starts at the first `#[cfg(test)]`,
  `#[cfg(all(test, …))]`, `#[cfg(any(test, …))]` or `#![cfg(test)]` line and runs to EOF. Still outside it:
  a multi-line `#[cfg(\n test\n)]`; a test module whose body lives in another file
  (`#[cfg(test)] mod tests;`); a test declared *above* the marker; and a marker that appears only inside a
  raw string (which makes the region start at the string). A determined author can therefore weaken a test and
  still pass the region comparison.
- **The check cannot tell which file the test lives in.** `test_path` is what it freezes; the runs use the
  `filter`. A decoy file whose module merely *mentions* the test name used to satisfy both — the `fn <filter>`
  requirement narrows it, but a filter that names a module rather than a function (as one branch's does) cannot
  satisfy it, so this check is skipped for such entries rather than faked.
- **What actually carries the causal claim** are the run-based checks, which are harder to fool: the RED leg
  must fail *with the quoted assertion text* and a `FAILED` summary and no build error, with `running N tests`
  (N > 0); the GREEN and HEAD legs must report `ok. N passed` with N ≥ 1 and the behavioural marker; the
  ancestry checks tie both SHAs to the pushed ref; and for the integration kind a skip is refused outright.
  A test that never ran, or that was filtered to nothing, cannot satisfy those.

If a branch needs a *sound* frozen-test guarantee rather than a heuristic, the fix is structural: put the
regression tests in their own file (as the frontend and integration entries already do, where whole-file
identity is exact) rather than relying on region extraction.

## Honest limits

- The gate proves the *named* test's transition. It does not prove the test is a good one, nor that the fix
  is minimal — only that this assertion was failing before the change and passes after, with the test file
  byte-identical across the two commits.
- It runs on `push` to the five branches. With `workflow_dispatch` declared as well, note that dispatch
  resolves against the **default branch**, where this file does not exist (`HTTP 404: workflow not found on
  the default branch`) — a push is the only trigger available from a feature branch.
- A skipped matrix leg is not a pass. The `required` job asserts the matrix result, but GitHub counts a
  skipped leg as success; the same trap the RE gate has. The fix is the rust-lang `citool` invariant
  (see `docs/re/upstream-test-techniques.md`): make the graph itself data and test that the asserting leg is
  the pushed ref's.

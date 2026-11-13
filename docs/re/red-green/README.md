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
| `rust` | `cargo test --lib -- <filter>` | the whole file at `test_path` (repo-relative) |
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
   - **rust** — the regression module lives in its own file beside the fix
     (`src/interfaces/middleware/rate_limit/tests.rs`), declared from the parent as
     `#[cfg(test)] #[rustfmt::skip] mod tests;`, so the comparison is the **whole file**, exactly as for the
     other two kinds — no region extraction, nothing inferred from content. That is what "the test was not
     weakened" means, and it is strictly stronger than checking that the lines the test commit added are still
     present somewhere: byte-identity also rejects an added `#[ignore]`, a new `#[cfg]` gate, an early return,
     or a reworded assertion, all of which leave the original lines in place. The `#[rustfmt::skip]` is the
     price of keeping the moved body byte-identical: rustfmt reprints an out-of-line module as a crate root and
     would otherwise demand the body be outdented to column 0, so rustfmt no longer reformats those three
     files;
   - **the frozen file must be the file that fails.** Byte-identity alone is not enough: `test_path` could name
     a decoy (a comment-only file) while the real assertion lived elsewhere and the fix rewrote it. Two checks
     close that, and both were added after an independent auditor **executed** the decoy: the declaration
     pattern is line-anchored (`^[[:space:]]*(…)?fn[[:space:]]+<filter>[ (]`), so a mention in a comment no
     longer satisfies it, and each RED leg requires its own log to name the frozen file — the repo-relative
     path for `rust`/`integration`, the basename for `frontend`, whose reporter shortens paths. Verified
     against the delivered entries: all four declarations match exactly once, the comment decoy is refused, and
     the real RED logs name the file (7, 8, 10 and 73 hits);
   - **every manifest value's shape is enforced before use.** Each value is substituted into a shell command as
     a GitHub expression, so the reader validates it (`red`/`green` = 40 hex, `test_path` = a path-shaped string
     with no whitespace, `filter` = an identifier, `kind` ∈ the three kinds), and requires `test_path`,
     `expect_red` and `expect_green` to be non-empty. This is what makes the substitutions safe: an **empty**
     `test_path` previously turned the identity step into a silent no-op — no comparison, exit 0 — and a value
     carrying `;` executed shell inside the gate, because `;` needs no quote to end a statement and the
     quote check therefore never saw it. Both were demonstrated against an earlier revision;
   - for the filtered kinds (`rust`, `integration`) the manifest's `filter` must be non-empty and name a
     `fn`/`mod` declaration inside the compared file, so a filter that matches nothing cannot masquerade as a
     frozen test; a `frontend` entry has no filter (Vitest selects by path), and the check is skipped for it
     rather than run against an empty pattern — which would fail every frontend entry before its tests ran;
   - and the comparison runs twice: against the **fix commit** (the causal transition) and against the
     **pushed HEAD** (what is actually delivered), because a later commit could re-touch the test or the code
     after the fix and the transition would still read green;
5. a RED run that **exits zero** (the test passed on the unpatched tree);
6. a RED run whose failure is a **build** error (`error[E…]`, `could not compile`, `cannot find`, `unresolved import`, `linking`, `failed to select a version`) — that is not evidence of the behavioural defect;
7. a RED run with no `test result: FAILED` / `failures:` marker, or with `running 0 tests` (filtered to nothing);
8. a RED run whose output lacks `expect_red`;
9. a GREEN run that does not report `test result: ok. N passed` (N ≥ 1) or lacks `expect_green`.

## What this gate does NOT prove — its residual limits, stated rather than implied

The identity check is now **whole-file and unconditional**: every entry names a file that is entirely tests
(a sibling `tests.rs`, the frontend test file, `tests/…`), so there is no region extraction and nothing a file's
content can do to weaken the comparison. The earlier heuristic — and the two bypasses an independent auditor
executed against it — is gone with the inline modules that required it.

What remains true, and is not proven by this gate:

- **The `#[rustfmt::skip]` on three declarations.** The moved modules keep `#[rustfmt::skip]` on their
  `#[cfg(test)] mod …;` line, because rustfmt reprints an out-of-line module as a crate root and would otherwise
  demand the body be outdented to column 0 — which would destroy the byte-identity this gate proves. The cost,
  stated rather than implied: rustfmt no longer reformats those three files.
- **The filter ties the entry to a file, not to the runner's selection.** The gate requires the filter to appear
  as a `fn`/`mod` declaration inside the compared file, which defeats the decoy-path attack; it cannot prove that
  `cargo test -- <filter>` selects exactly that test — the RED/GREEN legs prove *some* test failed and *some*
  test passed under that filter, and the log is quoted so a reader can see which.
- **One test is one test.** The gate proves the named assertion's transition with the file unchanged, and — after
  the decoy fix — that the failure came from that file. It does not prove the fix is minimal, or that no other
  behaviour changed: the RE gate's differential is what covers that, and the two gates are separate required
  checks for that reason.
- **`red == green`, and pairs whose ancestry is only "some ancestor", are not rejected.** Neither can produce a
  green gate deterministically — the two legs would have to report contradictory exits from the same tree, which
  only a flaky test could do — so they are documented rather than checked.
- **A branch certifies itself with the workflow and manifest it ships.** Nothing in the run hashes those bytes
  across the branches, so "identical on all five" is verified out-of-band (the loop at the top of
  `delivery-verdicts.md`), not by the gate. A reviewer must run that loop.
- **A skipped leg is not evidence.** The trigger lists and the matrices are set-equal, so a push always has one
  working leg; the required job now also refuses a ref that is not one of the five, which closes the
  `workflow_dispatch` path to a vacuous green that an auditor demonstrated was reachable.

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

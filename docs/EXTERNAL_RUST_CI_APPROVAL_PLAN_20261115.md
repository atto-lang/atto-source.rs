# Independent approval gate for private Rust compiler CI mirroring

**STATUS: PROPOSED — NOT APPROVED — SOURCE EXPORT DISABLED**

This proposal does **not** edit the deny-by-default allowlist or transfer any
private Oreslang source. An authenticated repository administrator requested
token-free, exact-tree mirroring so that unavailable source-org GitHub Actions
minutes do not block Rust testing. The repository's checked-in policy
(`.github/private-ci-approved-repositories.txt` and
`.github/workflows/private-ci-policy.yml`) presently authorizes **zero**
cross-organization source destinations. Do not bypass this condition from the
GitHub connector, even when carrier runner availability is confirmed.

## Verified source-free runner evidence

- Candidate carrier: `agent-pontifex/atto-ci`
- Carrier repository ID: `1458328902` (private; administrator write access)
- Standalone, source-free admission branch:
  `ci/source-free-rust-runner-proof-20261115`
- Commit: `1a0dee80b579b5c693d112fff99cbb6ead1e70d5`
- Exact-tree SHA: `198e11200653d5ebf3001672c4354ea581fe32be`
- Workflow run: <https://github.com/agent-pontifex/atto-ci/actions/runs/43780636691>
- Result: **SUCCESS**; checkout, exact-tree verification, Rust test
  compilation and Rust test execution completed with nonempty steps.
- This proves runner execution at that time only. It does not prove remaining
  account billing allowance, private-code authorization, source-tree equality,
  any Oreslang semantic property, or readiness to merge.

Other carriers checked: `voxletra`, `benefactor-cc`, and
`scintilla-run` had recent zero-step failed jobs, while
`honeypot-r-us-test`, `athlet-o`, and `zed-pkg-test` had historical or
recent successful runs. This plan proposes exactly one destination because
broadcasting private source to multiple organizations is unnecessary.

## Proposed sources and precise identities

All three heads below were read from the GitHub connector and are separate
draft branches. They must be re-read immediately before any transfer:

| PR | Purpose | Head commit SHA |
| --- | --- | --- |
| #83 | AoS/SoA/AoSoA checked layout planner | `a7636596db8439bc0df7e137e432444d99a1d84d` |
| #85 | bounded AOT trace / layout observations | `bc27ef81fc879e3694ff935bd177a1b1219f1801` |
| #87 | bounded arena with owner/generation retirement | `2138d5ddef9d75a812f1997487fd23286b4262a5` |

**Do not** combine unrelated draft trees, infer a tested head from a branch
name, or reuse a historical SHA after any branch advances.

## Required independent security review before export

1. Review an explicit, source-owned change authorizing the exact
   `agent-pontifex/atto-ci` *repository ID*, read-only compiler-test purpose,
   one source PR head, defined data retention and post-test cleanup.
   The existing deny-by-default check may be replaced only by a tested,
   fail-closed allowlist gate as part of that reviewed change.
2. Obtain independent approval from a maintainer/security reviewer.
   Confirm the carrier remains **private**, destination membership and
   permissions are understood, and no secrets, production data, credentials,
   git history, unused paths or license-restricted dependencies are exported.
   Review this repository's GitHub Actions / app permissions separately.
3. Enumerate the complete source tree at one exact source **commit**, and
   calculate each `path, mode, type, blob SHA`, including symlink entries.
   Reject submodules, unexpected LFS pointers, oversized/unsupported blobs,
   malformed trees and file paths that may leak secrets. Do not assume a
   tree `sha` can be reused across repositories without creating its blobs.
4. Recreate those approved blobs and modes through the GitHub connector
   inside a new, temporary branch in the *approved private carrier repo*.
   Its local `GITHUB_TOKEN` checks out only that carrier tree; no
   cross-organization checkout, PAT or deployed secret is used.
5. Compare **every** source and carrier blob SHA and relative path, including
   symlink modes, before running tests. Bind the CI workflow to the carrier
   commit, expected source SHA and expected exact Git tree SHA. Reject any
   difference or missing file; verify a clean worktree, then run:
   `cargo fmt --all -- --check`,
   `cargo clippy --locked --all-targets -- -D warnings`,
   `cargo test --locked --all-targets`,
   and native-object link/execute conformance fixtures.
6. Record nonempty job steps, exact source/carrier commit/tree SHA, toolchain
   versions, test counts, failures and logs. A runner canary, checkout
   success, queued workflow, or zero-step failure is **not** test evidence.
7. Delete the temporary branch/artifacts after retention review and verify
   no unintended copies remain. Deleting a branch does not immediately purge
   Git objects from remote storage; approval must account for that residual
   exposure. Do not mark the source PR ready solely on carrier host tests.

## Current blocker

The allowlist remains empty, the policy remains deny-by-default, and **no
private Rust source has been mirrored** in this investigation. Until the
explicit policy change passes independent review, run only source-free
admission tests in alternate organizations. PRs #83/#85/#87 remain draft and
their Rust code is not certified by the source-free runner success.

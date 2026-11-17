# Private exact-tree CI export: authorization review (NOT APPROVED)

Source repository: `ores-truffle-oreslang/oreslang-source.rs` (numeric repository ID `1426284360`, private).
Proposed destination: `benefactor-cc/atto-ci` (numeric repository ID `1458316385`, private).
Alternative destination, not automatically authorized: `anticaptrad/atto-ci` (numeric repository ID `1458325025`, private).
Initial candidate: PR #44 at `f1ff4e166a09ab517783f64710f684e12825f3d6`. Revalidate head at execution time.

## Current admission status

**DENIED.** `.github/private-ci-approved-repositories.txt` contains no active approved destination. Do not export source, upload Git objects, or insert an active destination entry until an independent reviewer explicitly authorizes this exact private repository and numeric ID. The current deny-by-default workflow deliberately rejects active entries; merely editing the allowlist is insufficient.

## Proposed reviewable implementation gates

1. Independently review this proposal, source sensitivity, destination owner access, retention, and repository visibility. Record approval and destination numeric ID; reject name-only matching, renames, public destinations, and forks.
2. Implement a dedicated, separately reviewed source-side authorization mechanism (the existing workflow explicitly rejects any active entry). Fail closed if the approved numeric ID, repository owner/name, source repository ID, source PR head, or tree differs from the review record. Never bypass branch protections or use elevated workflow tokens.
3. Verify the target organization actually has usable GitHub Actions capacity with a **source-free** canary first. A queued/failed job with zero executed steps is NOT_EXECUTED, not a test result.
4. Before copying any private content, read the current PR head commit and full recursive tree; reject truncated trees, submodules/symlinks or unsupported binary blobs until their handling is reviewed. Require exact source blob SHA, path, mode, and tree identity; never substitute default-branch files.
5. Create a temporary **private** destination branch with a parentless snapshot, preserve exact source tree as an isolated subtree, and verify the subtree Git tree SHA matches the source tree SHA. Any CI wrapper must live outside the source subtree and must not be reported as part of the source tree. Verify the source HEAD SHA, source tree SHA, destination subtree SHA, and per-blob SHAs before execution.
6. Run real Rust build/tests on a funded destination runner. Pin workflow/actions, disable checkout credential persistence, restrict permissions to contents:read, and record run ID, attempt, OS, job ID, executed steps, test outcomes and exact source SHA. Never equate a source-free canary or host model with guest/compiler proof.
7. Require independent review and original repository branch-protection checks before merging. Alternate CI evidence is supplemental and does not automatically satisfy protected source checks. Delete the temporary destination branch and artifacts under an approved retention policy.

## Observed CI state

PR #44 new-head runs for the private CI policy, runner admission and Rust compiler checks all completed with failure and **zero executed steps**. Their code-test status is NOT_EXECUTED. This proposal does not assert Actions budget exhaustion; the runner admission cause still requires confirmation.

## Negative tests required before enabling any export

- Destination owner/name matches but numeric repository ID differs: deny.
- Approved repository changes visibility to public: deny.
- Source head advances after authorization: deny until re-reviewed or pinned according to policy.
- Recursive tree response is truncated or a blob SHA differs: deny before triggering Actions.
- Runner canary has zero executed steps: do not export source or mark tests passing.
- Cross-org token cannot access private source: do not inject secrets; use only approved exact-tree transfer.
- Mirror tests fail or lack executed steps: keep PR blocked, report NOT_EXECUTED/FAIL honestly.

This file is a proposal, **not** authorization and not a test receipt.

## Independent carrier admission observations (2026-11-17)

The proposed carriers have **not** demonstrated available Actions capacity:

| Carrier | Examined run | Job observation | Admission |
| --- | --- | --- | --- |
| `benefactor-cc/atto-ci` | `43921171507` | Ubuntu and macOS jobs both failed with zero executed steps | NOT_EXECUTED |
| `anticaptrad/atto-ci` | `43983775257` | `real-compiler` job failed with zero executed steps | NOT_EXECUTED |

These are existing carrier runs, **not** tests of Rust PR #44. Their failure reason is not established; do not claim budget exhaustion from the job result alone. A usable alternate carrier must first show a source-free canary with at least one executed step and successful completion. Do not mirror any private source into either carrier while the source export policy remains denied.

### Authorization and tree-check implementation cautions

- A Git tree SHA verifies the contents, paths, modes and nested trees, but not the original commit identity. Record the original commit SHA separately; the destination snapshot commit SHA will differ.
- GitHub's Git Data API does not automatically transfer blob objects between repositories. Fetch each blob from the authorized source and create it in the authorized destination, verify each returned blob SHA, and construct the tree using exact modes/types. Never assume source blob SHAs are resolvable in the destination.
- A subtree path prefix necessarily changes the enclosing tree SHA. Compare the destination **subtree** SHA with the source root tree SHA, not the destination top-level tree SHA.
- Ensure no source-provided workflow can run with elevated permissions in the destination. Audit `.github/workflows` and the destination's active workflow triggers before the branch is pushed; isolate the test harness and do not execute untrusted PR workflows.
- Reject Git LFS pointers unless the referenced object content is separately approved and verified; matching pointer blob SHAs alone does not verify LFS payloads.

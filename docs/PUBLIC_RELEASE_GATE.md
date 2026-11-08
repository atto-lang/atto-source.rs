# Rust/Wasm browser compiler: guarded integration and publication

## Purpose

This integration branch folds the original Rust/Wasm compiler and semantic corrections into one **reviewable diff against `main`**. Earlier stacked PR #3 contains removed-operator and short-circuit semantics that have an outstanding changes-requested review. The integrated tree includes the **later fixes** and must be reviewed **as a whole**. Successful tests on a descendant commit **do not approve** an obsolete base PR.

## Source review gates (must all pass)

1. Independent review of the combined tree, including the original PR #3 changes requested and subsequent corrections. Reviewer must be different from the author; approval applies to the exact reviewed head.
2. Recheck that `%`, `^` and `^^` reject as `E_UNSUPPORTED`, and conditions `,` and `|` eagerly evaluate both sides left to right.
3. Verify `define class Point as ... end` plus `new Point()` executes on the **native Rust and wasm32-hosted Rust compiler**, while bare `Point()` rejects as `E_NEW_REQUIRED` and emits no guest Wasm.
4. Run actual Wasm guest execution, byte-for-byte native/wam32 output comparison, real Chromium worker E2E and class heap exhaustion/overflow negatives.
5. For the final PR head after any changes or merge, record Git head + tree, nonzero executed GitHub Actions job steps, source-backed acceptance counts, compiler artifact SHA256, ABI and actual compiled guest evidence. `steps=[]`, `steps=null`, runner zero or missing jobs are admission failures, not tests.
6. Keep Java differential work explicitly **not hosted** unless a real exact-head Java-backed result and receipt is present; do not claim five-backend parity from one compiler.

## Public `/play/` release gates (not authorized by CI alone)

The compiler's private source CI artifact is a release *candidate*, not deployment authorization.

- Require independently approved compiler source, reviewed immutable `atto_compiler.wasm` bytes, the source head/tree, artifact SHA256, compiler ABI and companion playground PR, and a reviewed, exact-byte pin in the playground.
- At the **site** (`atto-lang/atto-lang.github.io`), resolve [issue #5](https://github.com/atto-lang/atto-lang.github.io/issues/5) before publication: the current branch-based Pages source can publish automatically **even when the custom approval-gated workflow is skipped**. Repository admins must switch Pages publishing to GitHub Actions or disable Pages, enforce the `github-pages` environment independent reviewers and no bypass, protect `main`, and ensure only the approved manual workflow has `pages: write`.
- Prove the negative path: pushing changes to `main` does **not** publish; an unapproved manual dispatch does **not** publish. Prove the positive path only with independently authorized release reviewers and exact SHA256. Do not copy new candidate compiler bytes to a public branch or Pages before the guard is verified.
- Verify deployed `/play/` from HTTPS, including compiler binary digest, `application/wasm` MIME, CSP, GET-only source handling and real `new Point()`, `Point()` rejection, enum values, guest heap exhaustion and worker recovery.

## Current technical boundary

This is an intentionally narrow nominal-class compiler subset: explicit `new Name()` allocates a fixed default-value, immutable `int`/`bool` record in a single bounded 64-KiB guest Wasm memory. It does **not** implement actor-owned heaps, actor lifecycle, class methods, general constructors, mutation, reclamation or cross-backend class ABI. Empty actor declarations are syntax-only. Browser **compiler** workers are not AttoLang **actors**.

## Merge policy

Either merge approved stacked PRs in dependency order (#3 → #4 → #6 → #11 → #13), **or** approve and merge the consolidated integration PR directly to `main`, but **not both**. Prefer the consolidated PR when the original PR #3 still carries unresolved changes-requested review against an obsolete tree. Never interpret this integration branch as an approval for the separate playground or site.

## GitHub administrator protection and independent reviewer roles

The complete compiler PR #28 and the frontend PR #21 now have explicit
CODEOWNERS (`@oresoftware` and `@the1mills`); GitHub must independently
enforce required code-owner review via an active `main` branch ruleset.
CODEOWNERS alone does not require approval. A PR author/pusher's own
approval never substitutes for independent review. Use at least one
authorized non-author approval, dismiss stale reviews on changes, and
require approval after the last reviewable push. Protect `main` against
force pushes, deletion and unauthorized admin bypass; require the actual
`real-compiler` exact-head job.

The combined-main candidate #28 and original layered PRs #15/#18/#20/
#22/#25/#26 overlap; select exactly one merge path with independent
review and re-test the resulting main tree. PR #19 scalar-v0 actor ABI
is separate from the integrated browser v1 actor runtime; do not silently
merge it into this combined candidate.

The public site release workflow now fails closed on any approved-release
dispatch when its branch protection/rules, Pages workflow-only+HTTPS,
or `github-pages` environment policy cannot be established. This is
only an additional preflight; administrator bypass review and explicit
compiler artifact disclosure authorization remain mandatory. The
private compiler candidate artifact must not be published by this CI.

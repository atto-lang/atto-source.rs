# Exact /play language preview corpus (35 published sources)

This candidate pins the exact UTF-8 contents and Git blob SHA-1 of **all 35**
language-preview samples published in `atto-lang.github.io` at commit
`78e8320034f50f6d6f2b06b3301717d373d19932`. The snapshot is in `tests/playground_previews/`.
The manifest is a byte-fidelity check, not a declaration that the syntax
is currently supported.

`cargo test --locked` runs the native source-to-Wasm compiler on every
sample and requires deterministic guest output or a positioned diagnostic.
`ATTOC=<native-attoc> node tests/conformance.mjs` runs the same exact sources
through the **real Rust compiler built for wasm32**, verifies no guest artifact
on rejection, checks emitted guests and compares native/Wasm compiler bytes.
The already-supported functional pipeline sample is compiled and **executed**
with expected stdout and main return value 121.

The result JSON classifies every exact source as `unsupported` or
`admitted`; only a sample in `guestExecuted` has an actual positive
execution check. Other admission is explicitly unverified, never a success
claim for the feature. `parityCertified` is false until every feature has
positive and negative compiled-guest tests, browser Worker integration,
ownership/lifetime/exception cases, plus matching Java/LLVM reference
execution where those backends actually implement the construct.

Reference documents examined: Java `docs/PURE.md`, `docs/TRAP.md`,
`docs/USING_BORROW_SCOPES.md`; LLVM
`docs/CRITICAL_CALLABLE_SEMANTICS.md`, `docs/BORROW_ALIAS_MATRIX.md`
and `docs/TRAP_EXPRESSION_UNWRAP.md`. These distinguish design,
parser/AST or native runtime substrate from fully source-executable code.

This branch must not change or publish the website's pinned
`atto.browser.guest.v2` compiler, enable disabled previews, or claim
that source acceptance on one backend establishes cross-backend parity.
Website/site tracking: https://github.com/atto-lang/atto-platform-conformance/issues/31

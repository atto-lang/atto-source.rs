# Exact /play language preview corpus (42 published sources)

This candidate pins the exact UTF-8 contents and Git blob SHA-1 of **all 42**
language-preview samples published in `atto-lang.github.io` at candidate commit
`971366eed2f0b02673f6f6860a8cfbf526216e3e` (website draft PR #146). The current
published site still has the old private-method source; do NOT count the
14th public preview until the site correction has passed its own gates
and the website candidate has merged.. The snapshot is in `tests/playground_previews/`.
The manifest is a byte-fidelity check, not a declaration that the syntax
is currently supported.

`cargo test --locked` runs the native source-to-Wasm compiler on every
sample and requires deterministic guest output or a positioned diagnostic.
`ATTOC=<native-attoc> node tests/conformance.mjs` runs the same exact sources
through the **real Rust compiler built for wasm32**, verifies no guest artifact
on rejection, checks emitted guests and compares native/Wasm compiler bytes.
Fifteen currently supported previews have exact guest-execution oracles:
function-pipeline, generators, result-option, trap, borrow-checking,
lexical-scope-pure, state-containers, fib-match, guard, match-values, closures-functors, result-option-end, modules, callable-body-styles, and boolean-operators. All must be
compiled by the real wasm32 Rust compiler, executed in a genuine guest with
exact stdout/return values, and compared byte-for-byte with native Rust.
The other previews
are **not yet certified executable**. The modules profile supports direct
file-local qualified function calls only, not importing other files, module
values, generic members, or a cross-file visibility/linker model. Executing fifteen
samples does not establish complete ownership, actor, ADT or effect semantics.

The site has **nine** ordinary examples and **42** previews (**51 total**).
The nine ordinary sources have independent exact-blob and guest oracles in
`tests/playground_site_examples/manifest.json`; this corpus pins all 42
previews from current site commit 38a19bdc251d6f8db3ffea3b8df0e6c84ca8881c.
Fail-closed rejection is a safety property, **not successful implementation**.
The opt-in `ATTO_REQUIRE_ALL_EXAMPLES=1` gate deliberately fails until every
valid preview compiles and executes with a verified guest oracle. Do not
reclassify previews as runnable or promote compiler Wasm without verified
source-level execution and explicit release authorization.

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

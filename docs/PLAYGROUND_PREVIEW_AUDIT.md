# Exact /play language preview corpus (42 published sources)

This integration candidate pins the Git blob SHA-1 and exact UTF-8 source of all 42
website previews. The earlier `callable-body-styles.atto` public-access fix is
merged in website commit `67282439d97dcb76ec17b30cfa1177aad12c3ce7`.
The `boolean-operators.atto` source is pinned to the website correction
[merged PR #147](https://github.com/atto-lang/atto-lang.github.io/pull/147),
public source merge commit `ca3d55cd43470cd49cd6206b0e87f256112f84b9`,
which removes the reserved executable `^^` operator in favor of `Xor(a,b)`.
Website [PR #153](https://github.com/atto-lang/atto-lang.github.io/pull/153) (head
`faca5186d4d672ff5eaf843146e0baae3387a068`) then rewrote **comment lines only** in the
22 previews the public compiler executes, replacing stale "not browser-lowered" headers;
no declaration, statement or expression changed. All 42 Git blobs match that head.
The manifest verifies source bytes,
not executable support; this integration has not yet passed CI.

`cargo test --locked` runs the native source-to-Wasm compiler on every
sample and requires deterministic guest output or a positioned diagnostic.
`ATTOC=<native-attoc> node tests/conformance.mjs` runs the same exact sources
through the **real Rust compiler built for wasm32**, verifies no guest artifact
on rejection, checks emitted guests and compares native/Wasm compiler bytes.
Twenty-two candidate previews have independent guest-execution oracles:
function-pipeline, generators, result-option, trap, borrow-checking,
lexical-scope-pure, state-containers, fib-match, guard, match-values, closures-functors, result-option-end, modules, callable-body-styles, traits-interfaces,
diamond-inheritance, visibility-modifiers, routine, boolean-operators, match-types-is, data-shapes, and defer-recover. All must be
compiled by the real wasm32 Rust compiler, executed in a genuine guest with
exact stdout/return values, and compared byte-for-byte with native Rust.
The other previews
are **not yet certified executable**. The modules profile supports direct
file-local qualified function calls only, not importing other files, module
values, generic members, or a cross-file visibility/linker model. Executing twenty-two candidate
samples does not establish complete ownership, actor, ADT or effect semantics.

The site has **nine** ordinary examples and **42** previews (**51 total**).
The nine ordinary sources have independent exact-blob and guest oracles in
`tests/playground_site_examples/manifest.json`; this corpus pins all 42
previews from site PR #153 head faca5186d4d672ff5eaf843146e0baae3387a068.
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

The independently verified merged baseline remains 14/42 until this integration
passes real compiler/guest execution. The strict 42-preview release gate must
remain closed and the companion website correction must be coordinated before
deployment.

This branch must not change or publish the website's pinned
`atto.browser.guest.v2` compiler, enable disabled previews, or claim
that source acceptance on one backend establishes cross-backend parity.
Website/site tracking: https://github.com/atto-lang/atto-platform-conformance/issues/31

## Bounded scalar Any specialization (preview 20 candidate)

`match-types-is.atto` is compiled using a **strictly static, monomorphic
scalar specialization**: one immutable, directly literal-initialized `Any`
binding and one direct callable with an `Any` parameter are type-narrowed
to the proven scalar. Each `on is T` is checked against that scalar, and
unreachable type alternatives are removed before the ordinary match
checker/emitter; guarded matching is evaluated at runtime as usual.
This is **not** the general runtime-tagged `Any` object ABI: computed,
mutable, escaping, or multiply specialized `Any` values must reject with a
positioned diagnostic and without any guest artifact. A separate runtime
Any implementation is needed for arbitrary dynamic values.
The twentieth preview is **not certified** until exact-source Rust/wasm32
compiler byte parity, guest stdout, negative fixtures and Chromium E2E pass.

## Product-shape execution (preview 21 candidate)

`data-shapes.atto` is a **bounded source-local scalar product** profile.
Declared structs and records use checked guest-heap class constructors and
field accesses, while two-field `Tuple`, `Pair`, and `Entry` use distinct
private nominal classes with real runtime fields and canonical positional
printing: `(7, paid)`, `Pair(widgets, 2)`, `Entry(widgets, 2)`.
Keyed initializers must have an exact declared set of literal fields;
unknown/missing/duplicate fields, dynamic anonymous shapes, unsupported
generic arities, forged internal class identifiers, type mismatches and
external mutations all reject before guest emission. This is NOT the full
structural subtyping, record equality/copy, generic collection or
cross-actor transport ABI; those require separate runtime contracts.
Preview 21 is **not certified** until exact-source Rust, wasm32 compiler,
independent guest stdout/return oracle and Chromium tests complete.

## Deferred cleanup, recovery and escalated failure (preview 22 candidate)

`defer-recover.atto` executes the atto-spec
`language/defer-recover-unwind-v1.md` trace contract on legacy Wasm EH with two
module-private tags: tag 0 (ordinary `throw`) and tag 1 (`raise`). `catch
(Error e)` and `trap` match only tag 0, so `raise` always bypasses them.
`finally` runs once on fallthrough, on a pending return, and on any escaping
exception (catch_all + rethrow of the original exception). Return values are
snapshotted before finally/defer.

`defer` and `recover` register only at a `fnc` body's top level, so static
reverse source order equals dynamic LIFO registration order; per-registration
flags mean a cleanup registered after an early return never runs. On unwind,
a function's defers run before its recover callbacks.

**Two-phase catch precedence.** A guest-global active-catch counter (emitted
only when the program uses `recover`) is incremented by every active `catch`
and `trap` and restored from per-handler snapshots on every resumption path.
On an ordinary throw, a recover boundary first runs its defers, then rethrows
if any outer catch/trap is active, so a callee's recover never takes a throw
that a caller's catch or trap will handle. This is precise because the guest is
synchronous and this slice only admits the built-in `Error`, so every active
catch matches. Recover callbacks then run newest first; `c.next()` delegates to
the next older one; if all decline, the original tag and message escape.

Fail-closed limits: defer/recover bodies cannot see enclosing locals (no
capture lifetime proof), cannot throw/raise/return/try/defer, and finally
cannot return/throw/raise. `break`/`continue` cannot cross a handler. Methods,
actors and generators reject. The pending `err` and cursor are not
first-class values. `throws/raises/panics(Error)` clauses are syntax-validated
but not yet transitively effect-checked. Panics/engine traps bypass all
handlers and cleanup (the instance is dead). Programs without these constructs
keep byte-identical guests.

Preview 22 is **not certified** until CI-executed exact-source native and wasm32
compiler parity, `tests/defer_recover_guest.mjs`, and Chromium E2E pass on the
exact head, and a public compiler is promoted through the reviewed release gate.

# Design contracts: executable dynamic scalar requires/ensures v2

**Status:** proposed real parser, typed IR and Wasm code generation in [compiler PR #516](https://github.com/atto-lang/atto-source.rs/pull/516), guided by [spec PR #211](https://github.com/atto-lang/atto-spec/pull/211). **CI NOT EXECUTED**: no compiled guest or cargo test result is verified yet.

## Supported contract profile

The source parser preserves up to 64 requires/ensures clauses on ordinary functions and methods, but this first guest-executable profile is **top-level scalar functions only**; nominal class/actor/enum/collection source with contract keywords is blocked before emission. Requires must precede ensures, and result binders may not shadow parameters or self. The typed compiler accepts **boolean, readonly scalar expressions**, including comparisons and logical/ternary compositions over int/bool values. It rejects unsupported effects, calls, mutation, ownership operations and checked arithmetic in predicates rather than silently dropping checks. Constant true predicates are erased by proof; constant false is rejected.

The checks execute inside the callee, after argument binding and before its body. For dynamic ensures, the return type must be int or bool. Each ordinary return expression is evaluated **exactly once**, saved in a compiler-owned local, then checked by each postcondition before being returned. Nested if/block/loop explicit return paths are instrumented.

**Fails closed:** dynamic actor-method contracts, class/actor invariants, generator contracts, result types other than int/bool, early propagation through question-mark, try/catch/finally/defer/recover cleanup, iterator-owned returns, generic/module/visibility rewrites and unverified imports. Their runtime and ownership contracts are not yet implemented.

## Noncatchable runtime failure

The browser Wasm emitter emits distinct panic codes: 7 = failed requires; 8 = failed ensures. It stores this in the exported panic_code global, then emits the Wasm unreachable trap. The failure is not a guest throw tag and cannot be intercepted by an ordinary catch or trap. This is **not yet a typed ContractViolationError object**, actor supervisor CONTROL notice, or cross-backend parity.

A **module-wide** typed-IR gate also rejects contracts alongside any `try`, `finally`, `defer`, `recover` or owned `for-in` iterator cleanup, even if cleanup belongs to a different function: an uncatchable fatal contract trap must never silently skip mandatory cleanup in a caller. This prohibition will be lifted only after verified cleanup-aware panic lowering.

The compiler generates closed-world guest Wasm statically: no guest interpreter, runtime code generation, dynamic host reflection or silent fallback. Contract-bearing sources use the canonical parser → typed checker → Wasm emitter path directly, preventing specialized fast paths from losing contracts.

## Required validation

The focused Rust tests at tests/design_contracts_static_aot.rs compile real source programs and verify negative no-artifact diagnostics. The guest tests at tests/design_contracts_guest.mjs use the compiled attoc binary to generate Wasm, execute the guest in Node's native WebAssembly engine, and check return values and panic codes. The exact-head CI workflow runs both suites after verifying source and test Git blob IDs.

Until the relevant jobs execute with nonempty steps, report **NOT EXECUTED**. A zero-step failure is infrastructure blockage. A private alternate organization may run a token-free exact-tree mirror only if every source Git blob and the resulting tree SHA are verified.

The standard library assert module is independent of language-level requires/ensures; this change does not introduce a global catchable assertion builtin.

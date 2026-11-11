# Fail-closed atomic observer publication — draft V0 hardening

Borrow-checking is still performed in **one typed-CFG traversal**. The normal `verify()` path does not allocate a trace or re-run the graph. In the opt-in `verify_with_shadow_v0()` path, accepted operations are staged during the same analysis, and the callback is invoked **only after the entire acyclic CFG has produced a valid `Proof`**.

This prevents events from a seemingly valid earlier block from being interpreted as verified success when a later block fails with use-after-move, drop, alias conflict, invalid join or CFG structural failure.

## Limits and caveats

- Opt-in shadow publication has an independent **65,536-event** limit. Overflow returns `E_SHADOW_EVENT_BUDGET` **before publishing any trace**. The normal `verify()` accepts otherwise valid CFGs that exceed this trace budget.
- A consumer callback can fail after earlier callbacks have run; such external side effects are not rollback-able. On consumer failure, `Proof` is **never returned**. Downstream consumers must use the returned `Proof` as the only acceptance authority.
- The published trace is deterministic verifier analysis order (topological CFG processing), **not** a witness of which runtime branch executed. The typechecker must supply legitimate owner IDs, loan IDs, Copy/Send witnesses and source-to-MIR provenance.
- Java and LLVM partial hooks on the previous drafts still publish per-operation tentative callbacks. They need parallel staging hardening before their observations can be compared safely.
- No general loops, unwind, reborrow, pin-across-suspend, actor-tenant generation or full guest conformance is claimed.

## Verification gates

Run `cargo test --all-targets`, `cargo clippy --all-targets -- -D warnings`, and exact-tree verification on an authorized private runner. Add adversarial late-failure and budget tests. Source-org zero-step failed Actions are not test execution. No code change on this draft is certified by the earlier parent exact-head receipt.

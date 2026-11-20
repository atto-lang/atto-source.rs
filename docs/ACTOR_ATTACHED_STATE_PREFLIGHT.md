# Attached actor state: parse-only compiler boundary

**Status: SOURCE CHANGES COMMITTED; REAL RUST/COMPILER TESTS NOT YET EXECUTED.**

Canonical source target: [atto-spec #127](https://github.com/atto-lang/atto-spec/pull/127).
Portable tests: [atto-platform-conformance #113](https://github.com/atto-lang/atto-platform-conformance/pull/113).
Tracking: [atto-platform-conformance #114](https://github.com/atto-lang/atto-platform-conformance/issues/114).

This first compiler milestone parses `define state on actor Counter as { let int count; }` separately from class attachments and legacy actor fields. The parser records exact nominal actor owner, field name, scalar kind, writable `let` vs `val`, and whether an initializer was written. It validates the same-source actor forward or definition, requires one schema, checks field-name collisions, and rejects unclosed/empty schemas, duplicated names, untrusted kind impersonation and unsupported field types.

The initial scalar preflight accepts `int` and `bool`, a deferred `let int count;` (for **future** proof of definite initialization), and constant `:` or `=` literal initialization. **Deferred initialization is NOT proof that `on_start` initializes it.** Validation of that path requires lifecycle-aware checked MIR and remains blocked.

The main `compile()` entrypoint now recognizes attached *actor* state before ordinary parser/codegen. It intentionally returns `E_UNSUPPORTED` for a syntactically valid attachment. A malformed attachment returns a specific diagnostic. No partial state can silently lower into ordinary mutable fields or emit a Wasm guest. This is not full support for inline `state { ... }`, `using state.read/write`, `on_start` or real `receive`. Existing actor-local mailbox and class-container compiler paths remain independent.

## Next executable compiler milestone

1. Reconcile the production actor ingress with [Rust #297](https://github.com/atto-lang/atto-source.rs/pull/297) and the stateful Wasm callback work [#160](https://github.com/atto-lang/atto-source.rs/pull/160) / [#167](https://github.com/atto-lang/atto-source.rs/pull/167). Do not duplicate or silently merge their distinct constraints.
2. Introduce a checked `ActorStateSchema` (inline or attached) with exact nominal ownership and a lifecycle init proof. Conformity must be by actor incarnation, not process singleton or ordinary public class field.
3. Preserve `using state.read/write` through AST, typed MIR and guest instructions. Require actor identity, continuation/generation, scoped release on all paths, non-parking RW admission or equivalent proof, and no callbacks, awaits or raw state aliases under a live guard.
4. Implement `on_start` before `receive(ActorMail<int>)`; mailbox admission does not mean execution completion. Test the exact user Counter with two accepted turns and explicit drain/ack, checking 1 then 3 under serialized scheduling. Fault paths and tenant isolation require separate executed tests.
5. Reuse conformance #113 negative probes and real compiled guest runner. Supply source/head Git SHA, all blob hashes and executed-step evidence; a zero-step Actions failure is not a pass.

**CI:** `cargo test --locked --test actor_attached_state_preflight`, `cargo test --locked`, native + wasm32 compiler conformance and real browser Worker guest execution must run on an authorized exact-head environment before Ready/merge. Keep draft otherwise.

# Borrow shadow compatibility V0 — cross-backend review boundary

Canonical Rust draft: `atto-borrow-checker.rs` stacked on PR #1.
Java draft: `oreslang-source.java` stacked on #2691.
LLVM draft: `oreslang-source.llvm` stacked on #1615.

All three implementations now have **opt-in callbacks during the existing ownership pass**. The callbacks do not duplicate the source compiler's typed-AST pass or its ownership CFG traversal when disabled. The Rust core implements `verify_with_shadow_v0`; LLVM's `OwnershipDataflowV1::analyze(graph, sink, context)` reports accepted operation positions; Java `OwnershipChecker.checkWithBorrowShadowV0(program, sink)` reports select accepted moves, persistent borrows and suspension gates.

## Correctness restrictions

**These callbacks are not yet interchangeable verification inputs.** LLVM block/operation/owner indexes are local to its restricted CFG; Rust owner IDs are canonical only relative to the trusted typed `Program` input; Java emits diagnostic binding names and partial transition events. Rust currently has no proven frontend type/ownership provenance from Java/LLVM. Java does not emit all borrow-release and control-flow edge states. LLVM's source adapter still covers only the restricted two-return immutable-class slice.

Therefore:
- Never interpret an equal event name (e.g. `MOVE`) as cross-backend semantic parity.
- Never treat Java `SUSPEND_SAFE` or LLVM `ExitNormal` as proof of actor/Task continuation safety.
- Never grant Copy from Send or from partial trace labels. Trait witnesses must be independently typechecker-derived.
- A rejected operation must not be recorded as accepted. A shadow callback failure aborts proof.
- Every trace and compiled binary must be tied to an exact source head, canonical typed-IR identity, contract version, span mapping, and host/guest execution evidence before promotion.

## Next integration milestone

Add one **versioned typed ownership MIR adapter** to the Rust crate, with canonical owner IDs, disjointness/place roots, explicit loan identities, actor generations, Copy/Send witness hashes, edge states, effects and cleanup plans. Implement Java and LLVM *producer hooks in their existing passes*. Compute a canonical function fingerprint and share a cache across repeated checks. The Rust checker must verify data producers fail closed on all unknown statements/effects and must not accept arbitrary user-supplied trait bits.

On CI, replay the identical MIR through both independent verifier implementations and compare exact decisions and cleanup plans, then run actual .atto semantic negatives, real compiled positives, AOT and sanitizer gates. Non-executed Actions jobs remain `NOT_EXECUTED`.

## V0 tests

`cargo test --all-targets` validates the Rust callback contract (including NDEBUG-equivalent success/early-return rules on the C++ side). Run exact-head Java/LLVM tests and review the backward-compatible hooks before merging. This is **source-authored**, not proof of executed tests.

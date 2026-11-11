# atto-borrow-mir-v1: native fast paths, strict checker bridge

Status: **experimental library baseline; local model tests executed, NOT guest-certified**. See [integration evidence](INTEGRATION_BASELINE.md).

This draft adds `src/mir_v1.rs` to the existing [PR #1](https://github.com/ores-truffle-oreslang/atto-borrow-checker.rs/pull/1) acyclic canonical checker. The experimental field-sensitive cyclic CFG in [PR #2](https://github.com/ores-truffle-oreslang/atto-borrow-checker.rs/pull/2) is not silently merged or promoted; its Place/projection/loop work needs semantic reconciliation. No backend integration has been performed.

## Required backend pipeline

```text
source -> name/type/effect resolution -> typed HIR / typed MIR
             |                               |
             +-- local ownership constraints +-- compiler-built CFG
                                                  |
                                            native borrow pass
                                                  |
                        complete operation census + provenance/loan identity
                                                  |
                                          verified code-emission gate
                                                  |
            strict/differential CI ----------+-----+-- canonical Rust MIR checker
```

**Normal build:** Java, C#, C++ and target-native compilers collect ownership and type effects while constructing their typed IR, then run a native CFG analysis before emitting any guest artifact. Do not spawn a Rust process on every statement or function. The Rust checker runs as an independently implemented differential oracle in CI and strict/debug builds until a validated in-process equivalent is appropriate.

**Strict mode:** pin the exact compiler source Git SHA, input source SHA-256, generated typed MIR inventory, emitted instruction inventory, and semantic-contract version. Recompute hashes from trusted source bytes. Reject any missing, ambiguous, forged, skipped, stale or unexecuted evidence. A matching claimed 40/64-character digest is NOT cryptographic provenance.

## v1 bridge API and limits

`mir_v1::verify_mir(MirFunction)` accepts versioned `Producer`, entry block, CFG blocks and compiler-identified `Instruction` records containing `instruction_id`, `source_node_id`, `Effect` and a closed ownership `Op`.

`expected_instruction_ids` must come from a distinct census of validated typed MIR, not a copy of the event stream. The bridge checks uniqueness and set equality, effect classifications and malformed provenance *shape*, then invokes the restricted canonical ownership verifier.

`verify_mir_with_shadow_v0` uses the same envelope validation and then the transactional shadow verifier. Malformed inventories, invalid CFGs and budget failures produce no observer events; a consumer error prevents returning a proof, but consumer side effects cannot be rolled back. CFG edge and per-block instruction budgets are checked before allocating the lowered CFG copy.

The bridge **cannot independently establish**:
- that its caller derived `Copy` and `Send` from real type-checker facts;
- that the submitted MIR represents all source operations;
- that the compiler emitted only the validated operations;
- that the claimed hashes are the actual source, compiler or binary;
- that actor epoch/generation, FFI, unwind/throw/raise/panic, cancellation, pinned suspension, hot loading or partial-move destructors are safe.

Those obligations are explicitly NOT certified and their operations must be rejected at the adapter boundary. Never convert `Effect::Unsupported` to an ownership no-op merely to make the oracle pass.

The current V0 engine intentionally rejects loop cycles, divergent state joins, unknown effects, unbalanced loans, raw actor borrow hops and loans crossing suspension. These are **conservative rejects**, not permanent AttoLang language restrictions.

## Backend adapter acceptance

| Backend | Compiler placement | First integration gate |
|---|---|---|
| Java/GraalVM | Existing Java `OwnershipChecker` / typed-MIR construction | independently census predicate/guard `rt take` effects across true and false successors, export stable IDs, compare canonical diagnostics; Java PRs #2689/#2691 |
| LLVM C++ | ownership graph and typed-MIR -> LLVM emission | full LoanId/WriteId CFG-SSA and forward/reverse StoreInst/Use inventory; LLVM PRs #1556/#1610 and issue #1557 |
| .NET NativeAOT | C# typed CFG before CIL/native artifact | prove source objects, `Option` loans, yields, heap generations and static interop; .NET issue #419 |
| Forge C++ | existing native lowering | same versioned proof and exact-head compiled guests |
| Wasm | typed frontend, not JS runtime model | emitted guest bytecode ownership effects and worker transfer boundary |
| Cranelift/Rust | typed frontend before CLIF | link canonical Rust library in strict mode, avoid duplicate parsing |

## The minimum differential corpus

1. Shared/shared compatible; shared/exclusive incompatible; write-through requires unique mutable loan.
2. Use after move, drop after drop, Copy and Send independent trait evidence.
3. Predicate and guarded match move survives the false branch; only returning true-arm moves remain branch-local.
4. Move on one predecessor is a MAY-move at join; loan live on one predecessor is MAY-loan.
5. Whole actor-root transfer rejects raw overlapping borrow; no cross-tenant raw pointers.
6. Await/select with live unpinned loan rejects; explicit loan end before suspend passes.
7. Unsupported FFI, exception/unwind/cancel, critical-state access and cleanup effects reject before code emission.
8. LoanId/PlaceId/WriteId source-to-MIR and MIR-to-emitted use/store inventory must match independently counted operations.

Each fixture must distinguish `PASS_EXECUTED_GUEST`, `PASS_EXECUTED_COMPILER_NEGATIVE`, `HOST_ONLY`, `SOURCE_AUTHORED`, `NOT_EXECUTED`, `FAILED` and `NOT_CERTIFIED`. Test source-only model results are never upgraded to guest parity.

## Exact-head gate

```sh
cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
cargo test --all-targets
```

The source repository's hosted Actions billing/admission currently returns **zero-step failure**, not a Rust test verdict. Use only source-org remediation or an independently approved private carrier with exact Git tree/per-blob identities, no extra files and reviewed destination policy. Do not export private sources to unapproved organizations.

## Merge order

1. Run Rust exact-head steps and resolve compilation / lint results.
2. Review #1 with this stacked envelope and its regression tests, then reconcile #2's projections/loops into **one** canonical solver API rather than shipping competing checkers.
3. Add a real Java typed-MIR adapter plus independent source/emission census; then LLVM, .NET, Forge, Wasm and Cranelift.
4. Gate each backend on executed AttoLang semantic negatives, compiled positive guests, strict/AOT evidence, and independently reviewed exact-head provenance.

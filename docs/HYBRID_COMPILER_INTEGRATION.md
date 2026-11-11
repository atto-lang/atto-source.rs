# Hybrid in-pass + canonical borrow checker

**Design target — draft V0, not yet compiler integrated or release certified.**

## Decision

Every AttoLang backend retains a small, native-language incremental checker **inside existing compiler passes**:
- Java/GraalVM: Java `OwnershipChecker` and typed IR construction.
- .NET NativeAOT: C# type/CFG/CIL compiler passes.
- LLVM: C++ semantic resolution, CFG and IR generation.
- Wasm: its source frontend (not just JavaScript actor-arena runtime).
- Cranelift: Rust typed frontend/CLIF pipeline.
- Forge and other `oreslang-source.*` repositories are additional targets before a universal parity claim.

The Rust crate in this repository is a **canonical, backend-neutral verification implementation**. Compilers need not embed a Rust process on every source statement.

## Incremental compilation fast path

1. Resolve a canonical typed owner/place ID and compiler-attested, transitive Copy/Send/Share witness during ordinary type checking.
2. Update an ownership/loan lattice while traversing the same AST-to-typed-MIR and CFG construction pass. Cache by stable function IR fingerprint + rule set version + type witness digest.
3. If the function lies in an explicitly verified simple profile, permit in-pass checking without an additional out-of-process Rust invocation. The allowed profile must have differential equivalence evidence with the Rust checker; a local-only PASS is **never** automatically release-certified.
4. Otherwise send the canonical typed MIR function graph to the shared verifier through an in-process library/FFI bridge or invoke a cached worker in the compile daemon. If the shared verifier is absent, version-mismatched, or rejects, fail closed.
5. Run the Rust checker on every supported function under a shadow differential CI mode, compare exact diagnostic categories and cleanup plans, then build positive guest and AOT tests. A production fast path is enabled only after parity evidence on every feature it accepts.

### Performance

Avoid re-parsing, serializing ASTs as JSON on the hot path, repeated analysis of unchanged functions, and launching a Rust process per function. Use typed event streams or arenas. Rust can be built as a static library with a stable C ABI for LLVM; a managed C ABI bridge or sidecar is possible for .NET, and Java can use a native bridge or a persistent helper. Avoid requiring host-native FFI in browser Wasm by compiling a compatible Rust verifier to Wasm or using an independently proven in-pass adapter. None of these integrations is implemented in V0.

### Safety rule

If the Java/C#/C++ local algorithm and canonical Rust disagree, **never use the permissive result**. Raise a compiler-internal provenance diagnostic, preserve the source/IR fixtures, and block guest code generation. Independently validate provenance of nominal traits and source-to-MIR lowering. Never let a user construct trusted Copy/Send witnesses or arbitrary ownership MIR.

## Required next MIR features

V0 is intentionally **acyclic, ownership-root based and conservative**. Extend to versioned MIR V1 with:
- SSA/block parameter edge-specific ownership transfers and sound fixed-point loops.
- Lexical/field-sensitive place projections and disjointness proofs.
- Early returns, raise/throw/panic, catch/finally/defer/recover, and exactly-once cleanup.
- Authenticated actor and tenant domain generations, recursive Task captures, cancellation and hot-load invalidation.
- Pinned continuation loans that can cross await/select only with proven lifetimes, actor authority, stable allocation and correct resurrection/teardown fences.
- Foreign-function and ABI effect summaries, mutable state critical-read/critical-write gates, and resource/analysis budgets.
- Distinguish Sendable (actor transport) from Serializable (network encoding). No automatic transitive Copy from Send.
- Stable diagnostics, source spans and ownership proof receipts.

The present V0 `Traits` values are supplied by trusted compiler adapters but **are not themselves cryptographically authenticated proofs**. Direct consumers of `verify()` must not treat untrusted trait flags as proof. No native compiler call paths exist yet. The V0 verifier reports only bounded source-free CFG checks.

## Relation to existing work

Keep `oreslang-static-analysis.rs` as the broader effect, real-time and static analysis package; reuse verified CFG components where practical rather than maintaining two authoritative CFG semantics. Coordinate with Java #2687/#2689/#2691, LLVM #1598/#1610/#1615/#1627, .NET #418/#429, Wasm #444, Cranelift #97 and platform-conformance #646.

## Evidence and merge gates

`cargo test --all-targets`, `cargo clippy --all-targets`, and `cargo fmt --check` on an admitted exact-head runner; link SHA, step, artifact and logs. Run independent in-pass vs canonical semantic positives/negatives and executed .atto guest tests on every backend. This candidate is **NOT_EXECUTED / NOT_CERTIFIED** until those jobs execute. Source-authored tests and zero-step GitHub Actions do not count.

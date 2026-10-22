# Rust compiler architecture and verification status

| Layer | Technology | Status |
|---|---|---|
| Lexer | Logos | Working scalar subset |
| Lossless syntax | Rowan | Selected, not yet integrated |
| Incremental semantic graph | Salsa | Selected, not yet integrated |
| AST and semantic validation | Oreslang-specific | Signed i64, direct calls, const bindings, arity/name checks |
| HIR/MIR | Owned Oreslang abstraction | Scalar shell only (expressions still reuse AST nodes) |
| Native backend | Cranelift | Host object emission; parameterized functions and forward calls |
| Effect interchange | JSON schema v1 | Compute and static calls exported for Rust analyzer |
| Conformance | Shared Java/LLVM fixtures | 7 native and 5 diagnostic fixture classes in CI |

IR **must** eventually record explicit moves/borrows, actor allocation domains, immutable share boundaries, control-flow blocks, Task scopes, cancel/suspend edges, error propagation, real-time effects, stack maps and timing annotations. Cranelift lowers already-checked MIR; it is not the high-level language/ownership model.

The scalar exporter accurately covers only syntax admitted by today's Rust frontend; it is not a proof of runtime WCET. The separate Rust analyzer checks declared compute units and call graphs, not actual target CPU cycles. Generic integer operations currently follow emitted native i64 instruction behavior; do not claim overflow conformance beyond explicitly portable fixtures.

See \`tests/CONFORMANCE.md\` and \`docs/SELF_HOSTING_ROADMAP.md\`.

## Audited analysis boundary (v2)

The current compiler parses parameters, local bindings and direct calls; its
`--effects` mode emits schema-v1 `compute` and `call` operations from the
same validated MIR used by Cranelift. The audit adds a 1 MiB source budget,
4,096-token budget, 64-level nested syntax cap, 128-level/4,096-node MIR
expression cap and limits on functions/locals/parameters.

Both the publicly callable Cranelift emitter and effect-IR exporter now run
`mir::validate_module` before recursion, rejecting unresolved calls and forged
MIR. The resulting JSON is still **untrusted model data**, not cryptographic
source provenance or a hard-real-time execution certificate. Java/LLVM
cross-backend guest-conformance and timing/stack proofs remain separate gates.

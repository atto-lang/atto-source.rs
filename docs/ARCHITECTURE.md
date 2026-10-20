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

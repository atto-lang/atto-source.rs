# Oreslang bootstrap and self-hosting plan

This repo stays Rust-first while the source language and a small independent runtime become capable of hosting the entire frontend.

## Verification checkpoints

1. **Semantic definition:** pin common source fixtures from Java and LLVM, normalized diagnostic classes, canonical format and source spans. Each new Oreslang feature is admitted by parser/type checker and explicit MIR effects or rejected.
2. **Compiler infrastructure:** extend scalar AST to Rowan lossless CST with error recovery, typed HIR, and Salsa invalidation tests for incremental edits. No codegen on invalid HIR. All state/query keys must be deterministic.
3. **Ownership-aware MIR:** represent actor arena ownership, \`rt borrow\`, \`rt take\`, \`rt share\`, capability boundaries, suspend points, Task cancellation, exception edges, alloc/copy/control-flow cost nodes and stack roots before lowering to Cranelift.
4. **Runtime ABI:** freestanding \`core\`, optional \`alloc\`, and OS-dependent \`std\`; native object/JIT linking, AOT capability table, serializer and package metadata contracts; no mandatory OS heap for \`core\`.
5. **Independent verification:** compare Rust/Java/C++ AST/HIR effects and native outputs under seeded differential/metamorphic fixtures, fuzzing, and bounded resource budgets. A copied effect IR is not independent soundness evidence: independently instrument runtime allocations, message ownership transfers, actor isolation and suspension.
6. **Self-hosting:** implement Oreslang versions of lexing, parsing, MIR checking, and static analyzer, compile by the Rust bootstrap compiler, then reproducibly compile that same Oreslang compiler with itself. Verify stage-1/stage-2 binary reproducibility or normalized semantics, plus cross-backend differential tests, before declaring bootstrap complete.

Do **not** choose Cranelift IR as a language-level MIR. Rust, Java, C++ and eventual Oreslang engines should compare a versioned language-semantic contract without merging their runtime implementations. No step above currently constitutes self-hosting or a hard real-time proof.

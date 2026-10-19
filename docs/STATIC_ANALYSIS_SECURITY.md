# Rust frontend and codegen admission boundaries

This compiler is a deliberately restricted Cranelift scalar prototype.

## Current enforced limits (audit branch)

- Input source: 1 MiB maximum (CLI enforces before whole-file allocation).
- Lexer: at most 4,096 tokens.
- Parser: recursive expression nesting at most 64 admission levels.
- MIR: at most 1,024 functions, 4,096 expression nodes and 128 expression
  depth per function; codegen repeats MIR validation for direct public callers.
- Unrecognized syntax, duplicates and unsupported ABI entrypoints fail closed.

These bounds prevent stack overflow and excessive traversal in the initial
parser/codegen, **not** proof of execution resource usage. They do not certify
Oreslang cross-backend integer overflow behavior; direct Cranelift i64 arithmetic
currently wraps, and Java/LLVM source semantics must be reconciled before parity.

## Proof/refinement backlog

1. Pin a canonical source grammar, signed i64 and expression-evaluation ABI.
2. Replace the temporary hand-written AST with a typed HIR/MIR admitting full
   spans, control-flow joins and provenance-aware ownership.
3. Emit versioned, backend-neutral effect IR to the Rust analyzer with exact
   source-tree binding and fail-closed unknown calls/effects.
4. Compare compile/execute traces against Java/LLVM golden source fixtures.
5. Add negative fuzzing for deeply nested, malformed and adversarial syntax.

Passing `cargo test` on this prototype is not evidence of full source-language
correctness, actor isolation, zero-GC semantics or hard real-time guarantees.

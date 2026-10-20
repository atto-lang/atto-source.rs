# Cross-backend conformance contract (v1)

The three Oreslang compiler implementations are Java/GraalVM, C++/LLVM, and Rust/Cranelift. The language specification and golden fixtures are authoritative; no compiler's parser or backend is authoritative over another.

## Prototype fixture: scalar expressions

| ID | Input | Expected |
| --- | --- | --- |
| SCALAR-001 | `40 + 2` | native i64 42 |
| SCALAR-002 | `1 + 2 * 3` | native i64 7 |
| SCALAR-003 | `(1 + 2) * 3` | native i64 9 |
| SCALAR-004 | `9223372036854775808` | parse rejection |
| SCALAR-005 | duplicate static symbol | semantic rejection |
| EFFECT-001 | pure scalar function | explicit empty `operations` list, IR schema version 1 |

These are **Rust prototype fixtures** and must be reconciled with the canonical Oreslang syntax and arithmetic semantics before being enforced in Java/LLVM. No ownership, actor, allocation or real-time proof is implied. Do not mark unsupported syntax as passing.

## Next shared fixtures

Source locations, grammar recovery, borrow/move/escape, zero-copy `rt share`, actor domain permissions, async Task suspension, cancellation and quiescence, exception edges, method overloading, ABI/wire behavior, compressed pointers and SoA/AoSoA layouts. Run three independent compilers against the same fixtures, compare diagnostics and runtime values.

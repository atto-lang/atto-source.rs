# Rust conditional-expression v1 parity scope

Reference: shared exact contract from Java/GraalVM and LLVM at
`contracts/v1/conditional-expression-v1.json`, Git blob
`e3da61d86e556e801454029acef3bc4879d954a9`.

Implemented in the Rust **narrow scalar** compiler: bool literals, typed
`==` between matching int/int or bool/bool operands, and right-associative
`condition ? when_true : when_false`. The condition must statically be bool;
the two arms must share a type; both are statically checked but only one
executes. Non-void scalar functions, locals and call parameters remain int.
An `if` statement is not added in this PR; ternary is a typed expression,
not an implicit return from `if`.

Cranelift lowers the ternary to separate branch blocks and a typed SSA join.
Checked-i64 mode preserves lazy execution of unselected overflowing arms.
Regressions include the five exact shared scalar vectors, additional
positive native execution, unselected branch name diagnostics, branch
type mismatch, bad integer truthiness, and unsupported analyzer certification.

**FAIL CLOSED:** the existing effect IR v1, safety IR v1, flow v1, CFG v2,
paired compilation and static facts v0 lack complete branch-sensitive
semantics. They refuse typed conditional programs or return explicitly
`effects_complete=false`; they do not claim proof. The native compiler
only reuses the separate bounded AST admission checks.

Execution evidence: **NOT_EXECUTED** until actual Rust frontend -> MIR ->
Cranelift ELF -> native guest runs succeed on the exact PR head. Contract
source-byte equivalence is independently auditable from the copied JSON;
Java and LLVM native execution are **NOT_CERTIFIED** by a Rust-only test.
No private source is mirrored, no CI carrier permissions are changed,
and no full actor/ownership/Task/hard-RT parity is claimed.

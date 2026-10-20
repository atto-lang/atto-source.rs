# Oreslang Source (Rust) — independent compiler implementation

This is Oreslang's independent native compiler implementation built in Rust. It **does not** use GraalVM or LLVM: the current native code generator is **Cranelift**.

## Working scalar subset

* Canonical declarations \`pub fnc main(): int { return ...; }\` and \`fnc add(int a, int b): int { return ...; }\`
* Legacy bootstrap form \`static fnc answer() -> int { return 42; }\`, and \`as ... end\`.
* Signed i64 literals (including \`-9223372036854775808\`), numeric separators, nested and line comments, \`+\`, \`-\`, \`*\`, unary minus, parentheses.
* Lexically resolved scalar parameters, sequential immutable \`const local: int = ...;\` values, and direct, forward function calls.
* Scalar semantic admission: undefined names, mismatched call arity, duplicate functions/locals, oversized integer literals, unterminated nested comments.
* Native object generation, executable conformance tests, and effect IR v1 export consumed by the separate Rust static analyzer.

\`\`\`sh
cargo test --locked --all-targets
cargo run --locked --bin oresc -- --check tests/fixtures/forward.ores
cargo run --locked --bin oresc -- --effects tests/fixtures/forward.ores
cargo run --locked --bin oresc -- examples/answer.ores target/answer.o
cc target/answer.o examples/driver.c -o target/answer
./target/answer
\`\`\`

CI also links and runs **seven canonical Java/LLVM scalar source fixtures** and checks **five canonical diagnostic classes**. See \`tests/CONFORMANCE.md\`.

## Architectural choices

* **Logos** lexer (implemented); **Rowan** lossless CST (planned); **Salsa** incremental semantic queries (planned)
* Oreslang-specific AST, type admission, and expandable owned **HIR/MIR** (small scalar form implemented)
* **Cranelift** object backend (implemented), standalone JIT (planned)
* Separate backend-neutral \`oreslang-static-analysis.rs\` Rust analyzer, using versioned JSON effect IR

## Limits and milestones

There are no compiled actors, isolated arenas, borrow checking, async Tasks/await, exception supervision, full generics, portable freestanding runtime or hard-RT proof in this Rust compiler yet. Intermediate machine arithmetic overflow outside the portable no-overflow subset is not yet language-specified.

Read \`docs/ARCHITECTURE.md\` and \`docs/SELF_HOSTING_ROADMAP.md\`. Java/GraalVM and C++/LLVM remain independent implementations and existing reference contract producers.

# Oreslang Rust compiler

Independent Oreslang frontend, MIR and native code generation with Cranelift; neither LLVM nor GraalVM is a dependency.

This implementation deliberately begins with a **narrow executable subset**, not a claim of language completeness. A zero-argument static function returning an int64 arithmetic expression is tokenized with Logos, parsed into an AST, validated/lowered to MIR, and compiled by Cranelift to a native object file.

Quick start:

    cargo test
    cargo run --bin oresc -- examples/answer.ores target/answer.o
    cargo run --bin oresc -- --layout-ir examples/answer.ores
    cc target/answer.o examples/driver.c -o target/answer
    ./target/answer

Long-term choices: Rowan lossless CST, Salsa incremental query engine, owned Oreslang HIR/MIR, Cranelift native AOT/JIT. See docs/ARCHITECTURE.md.

The prototype currently accepts both the braced and as/end spellings for a static function. Its grammar and integer-overflow semantics need reconciliation against the Java and LLVM Oreslang language specification before broad use.


The Rust MIR also exposes a checked data-layout evidence boundary for the independent static analyzer. The current scalar subset has no aggregate regions, so --layout-ir emits a complete empty region list rather than guessing about SoA/AoSoA/ECS opportunities. See docs/LAYOUT-IR-V1-BRIDGE.md.

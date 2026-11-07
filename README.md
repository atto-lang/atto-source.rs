# Oreslang Rust compiler

Independent Oreslang frontend, MIR and native code generation with Cranelift; neither LLVM nor GraalVM is a dependency.

This implementation deliberately begins with a **narrow executable subset**, not a claim of language completeness. A zero-argument static function returning an int64 arithmetic expression is tokenized with Logos, parsed into an AST, validated/lowered to MIR, and compiled by Cranelift to a native object file.

Quick start:

    cargo test
    cargo run --bin oresc -- examples/answer.ores target/answer.o
    cc target/answer.o examples/driver.c -o target/answer
    ./target/answer

Long-term choices: Rowan lossless CST, Salsa incremental query engine, owned Oreslang HIR/MIR, Cranelift native AOT/JIT. See docs/ARCHITECTURE.md.

The prototype currently accepts both the braced and as/end spellings for a static function. Its grammar and integer-overflow semantics need reconciliation against the Java and LLVM Oreslang language specification before broad use.

## Rust-only tooling policy (no Python)

**Python is prohibited in this organization.** Do not add, maintain, invoke, generate, or require Python code or Python-based scripts, helpers, tests, code generators, build utilities, automation, CI steps, notebooks, or runtime dependencies. This includes `.py`, `.pyi`, `.ipynb`, `python` / `python3` commands, `pip`, `uv`, `poetry`, `pytest`, and Python GitHub Actions setup steps.

Implement tooling previously written in Python in **Rust** instead. Preserve behavior and test coverage while migrating; use `cargo fmt`, `cargo clippy`, and `cargo test` for Rust code. Existing Python must be inventoried and ported, not merely deleted if it implements required behavior. Changes introducing Python must be rejected in review and CI. References discussing the prohibition are allowed in documentation; they are not an authorization to execute Python.

This rule applies across all repositories, including compiler backends, examples, benchmarks, workflows, and documentation. Language implementations written in Java, LLVM/other native languages, .NET, etc. remain in their existing implementation languages; **the Python-to-Rust mandate applies to Python tooling and Python implementations**.

## Build profiles

The compiler accepts `--release`, `--strict-aot`, and `--hard-rt` as mutually
exclusive selectors, plus orthogonal `--freestanding`.

The current scalar subset contains no runtime reflection, heap/GC, dynamic dispatch,
dynamic code loading, blocking I/O, or loops. Release-or-stronger compilation performs
a semantic call-graph reachability pass before Cranelift and removes unreachable private
functions. Strict AOT uses the same closed world and emits only native object code.

Hard-RT admission is intentionally fail-closed: the compiler now proves a bounded,
acyclic call graph as a precondition, but rejects the final `--hard-rt` request until
target-specific WCET/deadline evidence is implemented. It does not relabel “native AOT”
as a hard-real-time proof.

## AttoLang scalar binding and declaration parity (draft, pending guest evidence)

The native Rust/Cranelift frontend accepts the canonical scalar `fnc`
declarations `define pub fnc`, `pub define fnc`, and legacy `fnc`.
The narrow sequential scalar body supports immutable-by-default
`int x = 7;`, `const x = 7;`, `const int x = 7;`, `const x: int = 7;`,
and `val x = 7;`. Explicit `let` and `mut` scalar declarations may be
reassigned with `=`, `+=`, and `-=`. Primitive rebinding lowers into
fresh, capture-avoiding SSA-style scalar definitions: no mutable borrow or
shared referent is inferred. Immutable bindings and parameters remain write
protected, and initializers execute in declaration order.

**Not implemented:** Boolean typing, `if/else`, `guard`, loops, `match`,
refinement types, arbitrary mutable objects, `rt` ownership operations,
actors, actor-local heaps/stacks, `critical`, `suspends`, async or channels.
Unsupported forms must fail compilation. The Cranelift backend's arithmetic
still needs independently verified overflow/trap parity with the Java/LLVM
contract; native object generation alone is not checked arithmetic proof.

Rust conformance tests include the exact canonical
`contracts/atto-parity-v1/add.atto` Git blob shared with Java/LLVM/Wasm,
negative binding tests, and Linux native-object link/execution checks. The
new native C ABI probe checks full signed 64-bit return values, not just
the lossy POSIX exit code. These are **declared test targets** until an
exact-PR-head CI run executes nonzero steps. Draft work must not be promoted
on source-only review or runner admission failures.

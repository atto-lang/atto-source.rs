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

## Repository ownership and migration (2026-11-08)

**Current home:** `ores-truffle-oreslang/oreslang-source.rs`. **Public language:** AttoLang (formerly Oreslang); retain legacy `oreslang-*` commands, packages, dependencies and existing `.ores` fixtures until each replacement is proven.

- **This repository owns:** Independent Rust-authored frontend, MIR, Cranelift native object generation and compiler tests.
- **Out of scope:** Claiming browser Wasm output, Wasmtime actor execution or replacing the separate `.cranelift` backend without evidence. Place shared language semantics and portable core/std in canonical `.atto` modules; keep only indispensable compiler, ABI and runtime-host primitives in this backend.
- **Organization boundary:** [`atto-lang`](https://github.com/atto-lang) is for language semantics, compilers, tooling, conformance and ecosystem libraries. [`atto-co`](https://github.com/atto-co) is for independently governed hosted products, deployment control, tenant infrastructure, identity and billing. Existing similarly named `atto-*` projects do not automatically share this repository's Git identity, code or test status.
- **First-stage migration proposal:** approved GitHub ownership transfer of this existing repo, preserving its name, numeric ID and history, to `atto-lang/oreslang-source.rs`. No direct legacy-org → `atto-co` transfer is authorized; subsequent product carve-outs require new approval.

**Migration instructions (maintainer preflight):** coordinate exact source head and ownership in [issue #26](https://github.com/ores-truffle-oreslang/.github/issues/26), then follow private [runbook draft #25](https://github.com/ores-truffle-oreslang/.github/pull/25). Confirm source/target admins and independent security reviewer, target access and branch/ruleset protection, exact SHA/tree, PRs/releases, Actions, secrets, GitHub Apps, deployments, dependencies and compatibility links before transfer. Do not clone/mirror private source as a shortcut. Verify unchanged numeric ID and code tree after real transfer, restore controls, rerun exact-head CI and capture **nonzero executed steps**. `steps=[]` / `steps=null` is `NOT_EXECUTED`, not test success; host/static checks cannot certify real AttoLang guest execution, AOT or zero-GC. Defer repository/CLI/extension renames to separately reviewed changes.

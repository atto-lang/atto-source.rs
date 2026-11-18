> **Canonical home moved (2026-11-16):** this compiler now lives in
> [atto-lang/atto-source.rs](https://github.com/atto-lang/atto-source.rs) at the native/ (subtree, full history),
> merged in atto-lang commit `60dbf3160d`. All commits up to `e0b8124c69`
> are preserved there. **Open new work and pull requests in atto-lang**; open PRs
> here must be re-opened against atto-lang before they can land.

# Oreslang Rust compiler

**Role:** Independent Rust + Cranelift compiler that currently outputs **native object files**, not browser Wasm32. This crate does **not** depend on Wasmtime and does not run source actors. The Rust-to-Wasm browser compiler actually shipped at `/play/` is **`atto-lang/atto-source.rs`**, a different project. See [compiler/host target boundary](docs/BROWSER_WASM_TARGET_BOUNDARY.md).

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

## AOT-first optimization planning (experimental)

`src/tiering.rs` provides target-specific *policy*, not a JIT compiler:
AOT is always the baseline; runtime JIT requests are rejected for browser
Wasm, WASI, embedded, strict-AOT, hard-RT, untrusted native hosts and native
hosts without executable-memory permission. Fixed-capacity trace counters
provide allocation-free hotness signals; a deterministic layout planner
recommends AoS, SoA or AoSoA for the next compiled representation, including
inverted-class layout hints. These primitives do not emit WebAssembly,
rewrite live object layouts or implement actor heap ownership. See
[docs/RS_AOT_TIERING.md](docs/RS_AOT_TIERING.md) and
`tests/tiering_policy.rs` for capabilities and negative-test boundaries.

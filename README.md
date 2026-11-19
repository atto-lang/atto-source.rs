> **Canonical home moved (2026-11-16):** this compiler now lives in
> [atto-lang/atto-source.rs](https://github.com/atto-lang/atto-source.rs) at the native/ (subtree, full history),
> merged in atto-lang commit `60dbf3160d`. All commits up to `e0b8124c69`
> are preserved there. **Open new work and pull requests in atto-lang**; open PRs
> here must be re-opened against atto-lang before they can land.

# AttoLang Rust native compiler (legacy Oreslang checkout)

## AttoLang migration and authoritative language contracts (2026-11-18)

**AttoLang** is the current language name (**formerly Oreslang**). This `ores-truffle-oreslang/oreslang-source.rs` repository retains the historical name and implementation context; the AttoLang-named counterpart is **[atto-lang/atto-source.rs](https://github.com/atto-lang/atto-source.rs)**, the destination for new AttoLang development. The legacy README already records a native/ subtree migration into the new repository. The browser Rust-to-Wasm compiler in the AttoLang repository and the legacy native Cranelift compiler are distinct; do not equate their features.

The **[atto-spec](https://github.com/atto-lang/atto-spec)** repository now owns normative language syntax and semantics, rather than this backend README, the old Oreslang repository names, or Java/Truffle behavior alone. Portable, positive **and negative** language tests belong in [atto-platform-conformance](https://github.com/atto-lang/atto-platform-conformance); compare implementations via [atto-compilers-parity-testing](https://github.com/atto-lang/atto-compilers-parity-testing). [atto-common-source](https://github.com/atto-lang/atto-common-source) owns portable `.atto` libraries. Backend-specific compiler code stays in its corresponding `atto-source.*` repository.

**Current source-design directions (targets, not claims this backend implements them):** canonical `.atto` for new portable source, `snake_case` function/method names, ownership operations `borrow!`, `take!`, `share!`, `copy!`, and `copy!!`; [implicit borrowing for nonprimitive arguments](https://github.com/atto-lang/atto-spec/blob/main/language/implicit-argument-borrow-v1.md) is still a design target subject to compiler verification. [File/module `define const`](https://github.com/atto-lang/atto-spec/blob/main/language/file-module-const-v1.md) supports proposed constexpr values **and** non-throwing, one-time runtime `os.read_env` initialization. [The proposed operator contract](https://github.com/atto-lang/atto-spec/blob/main/language/bitwise-and-stream-operators-v1.md) uses `<<<`, `>>>`, and `ashr` for shifts and reserves `<<`/`>>` for directional stream I/O; it is **not** automatically supported by legacy backends. [Shared ownership](https://github.com/atto-lang/atto-spec/blob/main/language/shared-ownership-and-guards-v1.md) does not grant unsynchronized mutable aliasing. Check spec documents for proposal/admission status before changing a parser or example.

**Compatibility and verification:** Keep existing `.ores` fixtures, `oreslang-*` commands, package IDs, fixed ABIs, and wire identifiers wherever still used; an AttoLang name change alone does not prove that a renamed CLI, syntax form, AOT target, actor isolation, zero-GC runtime or guest feature works. Retain exact legacy commands below when they describe this checkout. Do not use Python for project scripts, tests, CI helpers or tooling; use Rust for supporting tooling/tests where practical, without misrepresenting the backend implementation language. Record source commit/tree, target/toolchain, real positive/negative **compiled guest execution**, and `passed`/`failed`/`unsupported`/`not executed` for every claimed parity result. Host-only tests or workflows with zero executed steps are **not** compiler parity evidence.

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

## Legacy repository scope and contribution routing (2026-11-18)

**Historical checkout:** `ores-truffle-oreslang/oreslang-source.rs`. **AttoLang destination:** [`atto-lang/atto-source.rs`](https://github.com/atto-lang/atto-source.rs).

**Scope:** Historical Rust/Cranelift native compiler; the new AttoLang Rust repository also hosts a distinct browser compiler. The historical repository and new counterpart may not contain identical commits, pull requests, issues, or artifacts; a same-suffix repo is not proof of an in-place GitHub ownership transfer or identical Git history. Compare exact SHAs and migrate outstanding work deliberately; open new AttoLang design changes against [atto-spec](https://github.com/atto-lang/atto-spec) and new backend work in the appropriate AttoLang repository. The former plan to transfer `oreslang-source.rs` under its unchanged legacy name is superseded as a naming destination by `atto-source.rs` (without implying the old GitHub repository itself has moved).

Keep the [historical organization migration tracker](https://github.com/ores-truffle-oreslang/.github/issues/26) and [review runbook](https://github.com/ores-truffle-oreslang/.github/pull/25) for migration/security context, not as proof of a completed transfer. Preserve access controls, private source provenance, branch protections, PR/issue history and CI evidence; do not assume an existing `atto-*` repository is a verified byte-for-byte transfer. Host-only or source-free canaries do not prove guest code execution or parity.

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

## Fixed actor-owned arena (experimental)

`src/actor_arena.rs` provides an allocation-free, fixed-capacity Rust
slot store with actor/incarnation/arena identity checks, generation-fenced
handles, return-on-denial ownership and fail-closed capacity exhaustion. This is host-side substrate
only: it does **not** implement guest actor state, a general zero-GC
heap, source borrow checking or cross-actor memory isolation. See
[docs/RS_ACTOR_ARENA.md](docs/RS_ACTOR_ARENA.md).

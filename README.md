# Oreslang Rust compiler

Independent Oreslang frontend, MIR and native code generation with Cranelift; neither LLVM nor GraalVM is a dependency.

This implementation deliberately begins with a **narrow executable subset**, not a claim of language completeness. A zero-argument static function returning an int64 arithmetic expression is tokenized with Logos, parsed into an AST, validated/lowered to MIR, and compiled by Cranelift to a native object file.

Quick start:

    cargo test
    cargo run --bin oresc -- examples/answer.ores target/answer.o
    cc target/answer.o examples/driver.c -o target/answer
    ./target/answer

## Independent source-ID / object sidecar admission (candidate)

This compiler now has a **separate Rust-owned** source-identity verifier using its own
Logos parser, validated MIR and Cranelift native object generator. It does **not**
import Java/Graal or LLVM instrumentation code. This is routine metadata only;
log statements, span syntax, JIT probe insertion, OTLP, and actor context are
**not implemented** by this scalar subset.

Example (run on a machine with Rust and a native linker):

~~~sh
cargo test --lib source_identity
cargo test --lib independently_compiles_object_and_verified_source_map
cargo run --bin oresc -- --compile-with-source-ids \
  examples/source-ids-main.atto \
  examples/source-ids-main.registry.json \
  examples/source-ids-main.atto \
  target/source-ids-main.o \
  target/source-ids-main.map.json
~~~

### Review-only scalar routine registry sync

The Rust CLI additionally supports:

~~~sh
cargo run --locked --bin atto_telemetry_ids -- sync \
  examples/source-ids-main.atto \
  examples/source-ids-main.registry.json \
  examples/source-ids-main.proposed.json
~~~

The proposal must be a **new** canonical package-relative file. Neither the
original source nor the checked-in manifest is overwritten, and hardlink or
symlink destination aliases are rejected through exclusive creation. A kernel
CSPRNG supplies fresh nonzero 128-bit `rid_` identities on Unix platforms;
other platforms fail closed until an audited native CSPRNG adapter is added.
Existing identities remain unchanged across formatting and statement order;
the checked MIR verifier must accept every emitted proposal. The Rust library
also supports an *explicit whole-file move* with exact old/new callable
inventories, but the CLI does not expose moves yet. Renaming a function,
partially moving a module or inventing `sid_` sites must never silently
re-identify existing IDs. Do not confuse this experimental JSON interchange
with the final TOML manifest or a real compiled guest log site.

### Strict Rust source-ID authoring checks and lookup

The Rust-owned `atto_telemetry_ids` binary provides **compiler-checked**
validation and routine lookup; it does not require or invoke any other AttoLang
compiler backend:

~~~sh
cargo test --locked --bin atto_telemetry_ids
cargo test --locked --lib checked_routine_lookup_matches_independent_scalar_mir
cargo run --locked --bin atto_telemetry_ids -- check \
  examples/source-ids-main.atto examples/source-ids-main.registry.json
cargo run --locked --bin atto_telemetry_ids -- locate \
  examples/source-ids-main.atto examples/source-ids-main.registry.json \
  rid_a458b8330ea5475ba8a0cae2c64b48bc
~~~

The tool resolves both inputs inside the current package root, rejects
nonregular/symlinked inputs, and derives the logical source unit from its
canonical on-disk location rather than trusting a user-supplied name.
`locate` verifies the exact checked scalar MIR and can report a **routine ID
only**. It refuses `sid_` lookups because there is no compiled log-site lowering
yet. Its result says `verified_source_only_not_deployed_binary`; it is not an
artifact attestation. The final TOML authoring manifest and automatic `sid_` generation are **not
implemented**: source-controlled JSON remains the bootstrap interchange format.

The source-controlled JSON registry is a bounded candidate interchange form
for the proposed \`.atto.telemetry.ids.toml\` authoring surface. It requires
\`atto.telemetry.source-ids.v1\`, lowercase 128-bit \`rid_\` IDs, a package-relative
\`.atto\`/explicit legacy \`.ores\` source unit, and an exact callable signature.
Duplicate IDs, stale names, unsupported site mappings and source aliases are
rejected before code output. The compiled object and metadata sidecar are
produced from **one** validated Rust MIR; sidecar publication is not an
embedded probe or a stable binary ABI. Two output writes are not transactional.

**Audit hardening (2026-11-14):** The registry uses strict typed JSON
admission: duplicate object keys, unknown fields, duplicated IDs and
cross-source site-to-routine links are rejected. This scalar mode treats the
current working directory as the package root and requires the declared
logical `.atto` unit to match the exact physical input file. Source/registry
inputs must be ordinary regular files; on Unix the reader uses nonblocking,
no-follow descriptor flags and inode checks. Object and source-map outputs
use exclusive creation: existing paths, symlinks and hardlinks cannot be
silently overwritten. A failed second output or write can leave a partial
file; publication is **not transactional**, so consumers must require both
finished artifacts and authenticated digests before using a sidecar.

The Rust compiler and tests still need execution on a Rust-capable runner;
source-format and negative fixtures are authored, not proof of a completed
Cargo build. No actual `site_id` injection is implemented by this scalar path.

A routine ID identifies source code, not an invocation. OpenTelemetry
\`trace_id\` (128-bit) and \`span_id\` (64-bit) are runtime identities. A future
fully instrumented backend must preserve this distinction under inlining,
ownership/suspension, hot upgrade, and independently governed JIT or AOT paths.
**No cross-backend parity or executed CI is claimed by adding these files.**

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

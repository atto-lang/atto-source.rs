# Oreslang Rust Compiler Architecture

* Tokenizer: Logos (implemented).
* CST: Rowan 0.17 lossless, immutable green trees (selected, not yet integrated).
* AST/HIR: own Oreslang semantic types, overloads, pattern matching, names, symbols and actor ownership (small expression AST implemented).
* Incremental engine: Salsa 0.28 query architecture (selected, not yet integrated).
* MIR: own backend-agnostic Oreslang IR (initial scalar form implemented). Future MIR must preserve allocations, ownership/borrows, actor arena, zero-copy sharing, critical regions, Task await/suspension, exception edges, scheduler contracts, effect sets, stack bounds and traceability to source.
* Native backend: Cranelift 0.136 native object emission (implemented). No LLVM/GraalVM.
* Static analyzer: emit versioned effect IR consumed by oreslang-static-analysis.rs (planned). Analysis should remain useful with every backend.
* Diagnostics: stable diagnostic codes/spans; eventual Ariadne/SARIF/LSP (planned).

Cranelift IR is too low-level to act as the language-level MIR for ownership, concurrency and real-time effect proofs. Keep an independently specified MIR; validate it before lowering to backend instructions.

Roadmap: (1) this compiling/linking executable test; (2) grammar corpus / Rowan recovery trees; (3) type checking / Salsa; (4) ownership/async/actor MIR with lifecycle and JIT/AOT ABI; (5) runtime and cross-compiler differential tests; (6) self-hosting via Oreslang compiler bootstrapping.

Current limitations: named, zero-argument static functions only; one return expression; i64 operations; no userland calls, actors, async, heap or GC, debugger, linker, cross target or full standard library. Integer operations currently use Cranelift i64 arithmetic behavior; the source language's overflow policy must be finalized with conformance fixtures. A successful object emit is not a hard real-time guarantee.

## Audited scalar effect-IR exporter

`oresc --emit-effect-ir source.ores out.json` parses, validates and lowers the exact
same restricted scalar source subset as ordinary `oresc` compilation. It emits
`schema_version=1` JSON with every function and one explicit `compute`
operation (`units: 1`) per scalar MIR literal/arithmetic expression node.
Only integer literals and + / - / * are admitted; the resulting effect list
contains no **modeled** guest alloc/copy/I/O/FFI/call/actor/await effect.
`operations_upper_bound` represents abstract MIR-node work, not machine
instructions, cycles, stack or WCET. The exporter rejects programs whose
effect lists exceed the Rust analyzer's 100,000-operation budget. The source
language's integer-overflow policy, side channels, physical instruction count,
stack usage and WCET remain unverified.

The exporter is NOT a trust boundary: an attacker can forge JSON, and unrelated
backends must not claim the same effect closure without checked HIR/MIR lowering.
Production verification requires pinning the exact source tree, compiler version,
ABI, target and module dependency graph, and comparing emitted IR with authenticated
compiler traces. See the analyzer's `docs/VERIFICATION_OBLIGATIONS.md`.

### Golden cross-repository contract

`examples/answer.ores` must emit exactly
`examples/scalar-effect-ir-v1.json`, including byte-for-byte JSON
layout and SHA-256
`8ee1adbf6faef704c1eb6d2b2075c3b2d723097037f646f9e6e518a69d005463`.
The corresponding analyzer fixture is
`oreslang-static-analysis.rs/examples/rust-scalar-expected.json`.
Both repositories pin the same result independently without cross-repository
checkout or secrets.

**This is golden-fixture conformance, not authenticated provenance.**
A JSON file can be forged; a future protected compiler build must bind the
effect artifact to the exact source Git blob/tree, compiler image digest,
target triple, verified dependency closure and target-specific proof evidence.

### Reproduce the end-to-end two-repository check

With both private repositories checked out locally and Cargo installed:

```bash
python3 scripts/verify_scalar_effect_bridge.py \
  --analyzer-dir ../oreslang-static-analysis.rs
```

The script compiles the exact pinned `answer.ores`, produces v1 effect IR,
runs the Rust analyzer with `hard` profile, checks modeled units and the
explicit model-only assurance level, then runs mutation/negative controls
for unresolved calls, missing cost fields and zero-cost compute operations.
It does not fetch any repository or require a cross-repository GitHub token.

To check pinned source/IR fixture SHA-256 without Cargo:

```bash
python3 scripts/verify_scalar_effect_bridge.py --fixture-only
```

Both commands are **verification procedures**, not evidence of success
until they execute against the named Git commits. The CI runner for these
private repos currently fails before starting job steps.

## Same-MIR object + effect evidence (paired mode)

```bash
cargo run --locked --bin oresc -- --compile-with-effects \
  examples/answer.ores target/paired-answer.o target/paired-answer-effects.json
```

Both artifact types are generated from **one** checked MIR instance.
Effect admission completes before native object generation. The CLI rejects
input/output and object/effect destination aliases. The two file writes are
*not* atomic, and this does not authenticate the toolchain binary or dependencies.
The strengthened `scripts/verify_scalar_effect_bridge.py` runs a real
compiled C driver after the paired build, checks value `42`, checks the v1
analyzer's conservative work bounds, and performs malformed/unknown/zero-cost
negative controls. A passing analyzer `hard` profile remains **model-only**.

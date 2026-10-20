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
`schema_version=1` JSON with every function and an explicit empty guest effect
list. Only integer literals and + / - / * are admitted; zero operations means
no **modeled** guest alloc/copy/I/O/FFI/call/actor/await effect. The source
language's integer-overflow policy, side channels, physical instruction count,
stack usage and WCET remain unverified.

The exporter is NOT a trust boundary: an attacker can forge JSON, and unrelated
backends must not claim the same effect closure without checked HIR/MIR lowering.
Production verification requires pinning the exact source tree, compiler version,
ABI, target and module dependency graph, and comparing emitted IR with authenticated
compiler traces. See the analyzer's `docs/VERIFICATION_OBLIGATIONS.md`.

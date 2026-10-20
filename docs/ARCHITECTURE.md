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

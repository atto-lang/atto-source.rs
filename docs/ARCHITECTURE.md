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

## Parser and compilation resource limits (2026-10-19)

The independent Rust scalar prototype rejects source files larger than 256 KiB, more than 2,048 lexer tokens, and parenthesis nesting deeper than 64. The CLI reads a maximum of 256 KiB plus one byte before validation. These are deliberately conservative prototype limits to prevent unbounded recursive descent / MIR traversals, not final Oreslang language limits. Structured `resource_limit` parse failures are regression tested. The `--flow` exporter continues to reject calls until interprocedural effects are verified; it does not validate actor sends, borrow lifetimes or Task suspensions yet.

## Pure scalar call effect contract (stacked milestone)

The Rust frontend exports statically resolved scalar function calls in
`--flow` instead of unconditionally rejecting all calls. The checked MIR
already validates the direct target and argument arity; the standalone Rust
analyzer must independently validate each complete transitive callee effect
summary before any static admission. Arguments are evaluated first and
appear as their own abstract compute events; each direct call emits a
`call` event. No actor/Task/FFI/ownership syntax or compiler proof of hard
real-time execution is implied by this change.

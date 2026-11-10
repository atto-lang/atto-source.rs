# Browser target integration — 2026-11-10

Local execution evidence, not hosted CI or a production deployment certificate.

- Imported baseline: `atto-lang/atto-source.rs` head `b38b54a36c8024a2c77de006891341a2d846379f`, tree `6520a246849366dfdeb77eaba24956f56dc18713`.
- Canonical source now lives in this repository's `browser/` crate. Root `oresc --wasm` and `--js` call that crate directly; no external compilation server or native JIT fallback.
- Rust 1.85.1 built the import-free compiler for wasm32-unknown-unknown with 64 MiB maximum memory.
- `node tests/conformance.mjs`: 332 checks passed, including 150 native-versus-Wasm compiler comparisons and 150 additional ESM differential/rejection checks. This is the same frontend built for two hosts and two output formats; it is not independent native Cranelift language parity.
- Real Chrome headless: compiler and Wasm guest execute; emitted ESM executes two source-defined actors in one tenant Worker, with three distinct private Wasm memories (main plus two actors); overflow and recovery checked. Parent owns the deadline. Actor state is linear memory; JS temporaries and frames still use engine-managed memory.
- Root native Rust test suite passed, including CLI tests comparing both browser target outputs byte-for-byte to the core library. Native `.rs` actor code generation still rejects the actor source.

## Reference differences observed

Java head `6d8eb70048eafc9a84c5aebe9c1bb06104f16e76`, tree `f16af3c76a2a5e6927d760dc09bc9a13233e4d6d` was built using Maven and its real native thread library. The differential run reported **327 passed, 5 failed, 135 Java comparisons**. Do not suppress these differences:

1. `27-short-circuit.atto`, `47-eager-boolean-precedence.atto`, `48-while-eager-exit.atto`: Java short-circuits where this browser profile evaluates eagerly.
2. `67-actor-two-instances.atto`: Java's context-close race reports failure while actor output is in flight; the old prefix-based harness also mishandles permitted unordered output. This is not successful Java actor parity evidence.
3. `fail/14-untyped-int-binding.atto`: current Java accepts the declaration; this browser subset rejects it.

LLVM head `0ee037ca3e5dd9ac6ac1e9cc3d40f19ee6326a9c`, tree `6324f2fdbaf77d92e04e4b6148cf9b7adaa95217` built with LLVM 19. The unchanged `61-actor-state.atto` source rejects with `native actor requires pub on_message(int)` and emits zero IR bytes. Its actor surface must be reconciled with `receive(ActorMail<int>)` before parity can pass.

Independent `.cranelift` native tests passed, but the same actor fixture rejects with `DECL_KIND_UNSUPPORTED`; no actor guest is executed there. Neither this test result nor the `.rs` browser target grants native Cranelift actor support.

## Remaining gates

Native frontend convergence, modern sealed-state/concurrent-default actor amendment, independent Java/LLVM actor execution, source.js default-profile convergence, real browser deadline/aggregate-quota hardening, and review/merge/release of exact heads remain separate work. This target is the documented compatibility subset, not full-language certification. Wasm compiler and JS engine compilation are browser engine operations, not strict native full-AOT or zero-GC proof.

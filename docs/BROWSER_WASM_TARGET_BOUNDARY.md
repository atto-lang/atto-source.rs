# Native Cranelift compiler vs browser Wasm target

**Audited mainline boundary (2026-11-08).** `oreslang-source.rs` is a Rust compiler using Logos -> frontend/MIR -> Cranelift 0.136 -> **native object files**. `Cargo.toml` contains no `wasmtime` crate. It does not presently emit browser Wasm32 guests or run an actor host. `--strict-aot` selects a closed-world **native** compilation profile, not a `wasm32` cross-target.

Do not conflate three Rust roles:

- `atto-lang/atto-source.rs` is the **different Rust compiler** that GitHub Pages `/play/` vendors as `atto_compiler.wasm`. A browser Worker executes that compiled compiler, which emits a separate guest Wasm module. The checked-in Pages release is still pinned to `atto.browser.guest.v1`, not the current upstream actor v2.
- `oreslang-source.wasm` is primarily a **Node JavaScript/LLVM-to-Clang Wasm bridge**, with a separate native Rust `aot-host` using Wasmtime outside browsers.
- This repository has a distinct native Rust/Cranelift frontend/backend. It does **not** use Wasmtime or Web Workers today.

## Requirements for a future browser target

1. Expose an explicit `--target=wasm32-browser` profile backed by a real Wasm32 code generator/linker; renaming native object files does not count. Preserve canonical source HIR/MIR, ownership/effects and the same negative compiler semantics. Do not introduce a second shadow parser.
2. Version and verify browser guest imports/exports, source/compiler/artifact provenance, private wasm32 linear memory with a bounded maximum, typed ActorRefs, deterministic Sendable-only transport and quota-enforced actor frame/continuation arenas. No shared-memory or memory64 fallback.
3. Use a **module Web Worker per tenant** and **separate `WebAssembly.Instance` plus private `WebAssembly.Memory` per actor** within that Worker. Wasm32 pointers remain 32-bit offsets even on 64-bit hosts. Distinct arenas within ONE unrestricted Wasm instance are not independently protected against arbitrary Wasm reads/writes.
4. Enforce per-actor and per-tenant heap/stack/mailbox/CPU budgets. The engine's native Wasm call stack is not an actor-owned, quota-enforced stack. Prove compiler-managed frames or trampolining and bounded recursion. A synchronous loop monopolizes the entire Worker; terminating it retires **all co-located actors**, so parent-owned deadlines and generation fencing are required.
5. Verify a real `.atto -> wasm32 -> Chromium Worker` actor fixture (two instances in one Worker), fresh actor reincarnation, nonleaking state and memory, invalid ABI/import rejection, trapped actor/tenant failure scope, async cancellation, and differential parity with native and Wasmtime. Node host-model and synthetic Wasm tests are separate evidence. `steps=[]` is **NOT_EXECUTED**.

See `oreslang-source.wasm/docs/BROWSER_TENANT_WEBWORKER_V0.md` for the source-independent browser-host prototype and its limitations.

**Security:** Browser Workers are not independent OS tenant sandboxes. Untrusted mutually distrustful tenants need stronger origins/process isolation; Wasmtime native Store boundaries do not exist in the browser.
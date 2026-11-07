# M2.2 — Executable browser class objects (bounded primitive subset)

Supported: nominal class declarations, int/bool fields with primitive literal defaults, a default constructor or an explicit constructor with scalar arguments, direct methods with a hidden class-typed self, and self field reads/writes. Final slot writes are rejected.

Each new class object allocates a distinct aligned record in a non-growing 64 KiB guest Wasm memory arena, starting after static string data. The record includes an 8-byte class tag and 8-byte primitive slots. Allocation is checked before storing: exhaustion traps with panic_code=3. The memory is exported for guest inspection. No host imports or JS-managed object allocation are added.

Explicitly unsupported: reference aliasing, moving/rebinding class references, passing class handles to ordinary functions, returns of class references, external field access, nested class-valued fields, inheritance, dynamic dispatch, runtime borrow checks, rt borrow/take/share, actor isolation or mailbox messaging, destructors and per-actor allocation domains. Those cases fail before emitting guest bytes. Objects are reclaimed when the guest instance is destroyed; this is not independent per-actor memory ownership.

Run cargo test --locked, build the wasm32 compiler, then node tests/m2-object-wasm.mjs. Exact-head CI must prove real compiler-Wasm to guest-Wasm execution and negative controls before public promotion.

Public Pages is separately blocked pending security issue atto-lang.github.io#5 and independently reviewed binary-digest authorization.

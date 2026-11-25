# Opt-in instance-bound scalar receiver preview

This is a narrow execution milestone, **not** production AttoLang actor dispatch.
Ordinary `attoc input.atto`, browser `compile()`, and lease-bearing `mtd`
methods continue to reject with `E_UNSUPPORTED`.

## Compile

```sh
attoc --receiver-instance-preview examples/receiver.atto -o guest.wasm
# Or browser compiler: compile_receiver_instance_preview(source_length)
```

The source must pass the exact same immutable, lease-free, single-return
integer-expression grammar as `--scalar-preview`. The compiler emits
an import-free Wasm module with:

- `atto.bind.<Actor>(i64 token) -> i32`: binds a fresh module instance once
  to the nominal actor type and an opaque nonzero capability.
- `atto.receiver.<Actor>.<method>.<arity>(i64 token, i64 args...) -> i64`:
  requires active phase, correct nominal actor type and exact token,
  **before** evaluating the verifier-checked scalar CFG.
- `atto.actor_close(i64 token) -> i32`: erases the token and moves the instance
  into a permanently closed phase; further bindings/calls fail.
- `panic_code` (mutable i32): 1 for checked arithmetic overflow; 6 for
  invalid receiver token/type/lifecycle. Guest panics trap.

Each `WebAssembly.Instance` has private bound owner, token and phase globals.
The host creates **one Wasm instance per actor**; it must not pass these
capabilities directly to untrusted code. Never use a raw actor ID, an ordinary
incrementing counter, or a caller-provided token as the capability.

## Host adapter

```js
import {readFileSync} from 'node:fs';
import {create_receiver_runtime} from './runtime/receiver_instances.mjs';

const runtime=create_receiver_runtime(readFileSync('guest.wasm'),{
  on_control: notice => console.log(notice) // prototype hook only
});
const actor=runtime.spawn_actor('trusted-tenant-id','Worker');
console.log(runtime.invoke_method('trusted-tenant-id',actor,'double',21n)); // 42n
runtime.close_actor('trusted-tenant-id',actor);
```

The JavaScript host uses cryptographic WebCrypto randomness for 64-bit tokens,
a private WeakMap for opaque handles, exact tenant equality and terminal close.
It emits a **synchronous prototype panic notice** and closes the affected
instance on a Wasm runtime trap. The calling host must supply an **authoritative,
trusted tenant identity**, not unverified caller input. Tenant strings alone
are **not authentication** and a Wasm token alone is **not an unforgeable
cross-process security capability**. These are not scheduler admission,
priority CONTROL delivery or a production supervisor.

Missing: native direct actor dispatch, actual actor heaps/state, mailboxes,
scheduler/supervisor integration, cancellation and teardown propagation,
production lock leases and generated cleanup release attestation, secure
cross-worker capabilities, and independent Java/LLVM/.NET/Forge parity.
No suspension inside critical callbacks is enabled.

See `tests/receiver_instance.mjs` for executed binding, rejection, checked
overflow and native-vs-browser compiler parity.

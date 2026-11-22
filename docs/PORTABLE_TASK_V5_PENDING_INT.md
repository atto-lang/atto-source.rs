# Portable guest v5: real externally pending signed-64-bit futures

**Experimental, explicit opt-in, fully guest-AOT.** This milestone extends
portable-v4's private sixteen-frame Wasm arena with a real
`Pending -> Ready -> Resumed` external-completion lifecycle. It does not
interpret source, use JSPI, perform runtime guest code generation, or execute a
guest continuation in a MessagePort callback.

## Compiler admission

Native `attoc --portable-v5 file.atto -o guest.wasm`; Rust API
`compile_portable_v5(source)`; wasm32 browser compiler export
`compile_v5(length)`. The normal `compile()`, `compile_v4()`, v3/v4
markers, ABI exports and binary outputs remain unchanged.

**Constrained source profile:** exactly the previously accepted v3/v4
1–8 straight-line `await` sites, initialized local `int` operands or
literal integer operands, at most 8 integer locals, and explicit final
`return await`. In **v5 mode only**, an `await` operand is the signed-i64
*external future key*, not a yielded result. For example,

```atto
pub susp fnc main(): int {
  val first_key: int = 41;
  await first_key;
  return await 42;
}
```

awaits external keys 41 and then 42. The second completion's supplied i64
value becomes the function result. The first completion's value is consumed
but not bound in this constrained source subset. These are host-owned
**signed-i64 keyed pending futures**, not the general language
`Future<T>` generic type. There is no type-directed future constructor or
dynamic allocation syntax in this profile yet. Arbitrary branches,
captures, nested calls, error values and `Future<T>` specialization
must fail closed.

## Guest ABI

Version marker: custom section `atto.browser.guest.v5` payload `05`.
Existing `atto.async` marker `02` is retained, but the v5 host never
invokes `main` or JSPI. One private unexported, fixed 64-KiB Wasm memory
page; 16 independently allocated 96-byte frames. Handle is a positive
signed-i32 generation-checked value. Each frame contains owner, epoch,
monotonic non-wrapping generation, state/PC, signed-i64 final result,
pending i64 key, up to eight spilled i64 locals. 16-slot quota remains
strict and previously destroyed handles cannot be reused.

| Export | Signature | Meaning |
| --- | --- | --- |
| `task_start_v5` | `(owner:i32,epoch:i32)->handle:i32` | Allocate, compile-execute first segment, then wait |
| `task_pending_v5` | `(handle,owner,epoch)->key:i64` | Read current future key, traps unless waiting |
| `task_complete_v5` | `(handle,owner,epoch,key:i64,value:i64)->status:i32` | Only waiting and matching key accepted; 0 ready, -1 invalid |
| `task_resume_v5` | `(handle,owner,epoch)->status:i32` | Only after completion; 1 next pending, 2 done, -1 invalid |
| `task_cancel_v5` | `(handle,owner,epoch)->status:i32` | Pending/ready -> cancelled 4, else -1 |
| `task_result_v5` | `(handle,owner,epoch)->value:i64` | Only DONE, traps otherwise |
| `task_drop_v5` | `(handle,owner,epoch)->status:i32` | Erases payload and reclaims slot; leaves generation fence |
| `task_capacity_v5` | `()->i32` | 16 |

Each completion is keyed to the *currently pending* step. A second
completion or a premature resume is denied. Numeric guest handles/owner
IDs/epochs/keys do **not** authenticate a Worker; the trusted host must
check a separate unguessable single-use bearer, then call the guest
completion export from a scheduled quantum.

## JS host integration

`runtime/browser/portable-task-host-v5.mjs` is byte-for-byte shared
with JS backend PR #121. It uses the same `worker-completion-capability-v1`
implementation and a bounded control loop. Each task owns a separate
one-pending-at-a-time bearer registry, binding owner/epoch and a 128-bit
random capability. `host.spawn({owner,ownerId,epoch,onPending,port})`
returns a controller with `result`, `cancel()`, `complete(frame)`, and
read-only stats. `onPending({owner,ownerId,epoch,key,grant})` sends the
grant to the trusted completion Worker over a dedicated channel.

Completion handlers **only enqueue** the authenticated callback. A later
scheduler turn calls `task_complete_v5`, then *another* turn calls
`task_resume_v5`. No DATA turn is scheduled between registration and a
true external completion. Cancellation revokes the bearer synchronously,
prioritizes CONTROL cleanup, erases the guest frame, and rejects stale
replay; `close()` also reclaims frames after the scheduler is closed.
The bearer proves possession of a ticket, not the identity of a Worker;
holders can choose their authorized ticket's i64 result. The host must
authenticate its code and isolate malicious Workers separately.

## Required verification and unresolved features

Run `cargo test --locked --test portable_task_v5`,
`node tests/portable-v5-e2e.mjs` with a native-compiled `attoc`,
and the full native Rust/wasm32 compiler tests, compiled conformance and
real Chromium tests in private exact-tree GHA CI. End-to-end fixtures
require 16 pending tasks, no eager resumes, separately authorized
MessagePort completions, arbitrary completion order, value and key
distinction, bad-owner/key/replay rejection, cancellation, generation
fencing, deterministic source output and cleanup.

Not yet implemented: a source-level `Future<int>` constructor/type,
arbitrary `Future<T>` specialization, dataflow/CFG continuation spilling,
completion failure values, hierarchical TaskScope/supervisor propagation,
external result serialization, cross-backend compiler equivalence, and
contended `Mutex<T>.lock_async()`. Neither source PR should be merged or
promoted to the public playground solely on these experimental tests.

# Portable guest-v4 AOT continuation arena (experimental, opt-in)

## Scope and compatibility

This is the **first multi-frame Wasm continuation arena**, not generalized
AttoLang asynchronous execution. The existing v3 code path is unaffected:
`atto_compiler::compile(source)` and `attoc input.atto` continue to produce
the original v3 profile for the already-admitted source. Opt in explicitly:

```sh
attoc --portable-v4 task.atto -o task-v4.wasm
```

Native library entrypoint: `compile_portable_v4(source)`. The browser compiler
Wasm exports a new `compile_v4(length)` next to the unchanged `compile(length)`
with the same input buffer / result pointer / error buffer protocol.

The module advertises exactly one `atto.browser.guest.v4` custom section with
payload `04`, and keeps the existing `atto.async` compatibility marker `02`.
The v3 marker is **not** emitted for v4. The v4 guest has one *private* fixed
64 KiB Wasm linear-memory page (not exported), containing a preallocated arena.
Host code must not invoke the legacy `main` export on this path; any imported
`atto.cooperate` is a compatibility stub that should throw if reached.

## ABI: ordinary AOT-compiled synchronous exports

| Guest function | Result |
| --- | --- |
| `task_start_v4(owner:i32, epoch:i32)` | positive signed-i32 opaque handle; `-1` invalid, `-2` full |
| `task_resume_v4(handle:i32, owner:i32, epoch:i32)` | `1` PENDING, `2` DONE, `-1` INVALID |
| `task_cancel_v4(handle:i32, owner:i32, epoch:i32)` | `4` CANCELLED or `-1` INVALID |
| `task_drop_v4(handle:i32, owner:i32, epoch:i32)` | `0` READY/reclaimed or `-1` INVALID |
| `task_result_v4(handle:i32, owner:i32, epoch:i32)` | signed i64 only after DONE, traps otherwise |
| `task_capacity_v4()` | constant `16` |

No JSPI, `WebAssembly.Suspending`, `WebAssembly.promising`, interpreter,
runtime code generator or Promise continuation is used by these lifecycle
exports. A host scheduler invokes at most one guest export per synchronous
turn. Source-level tasks must remain explicitly owned by the host.

## Frame format, safety and limitations

The arena has 16 slots of 96 bytes each (1,536 bytes total), with signed i32
owner at byte 0, epoch at 4, non-wrapping generation at 8, resume state at 12,
signed i64 result at 16, captured await operand at 24 and eight potential i64
locals at byte 32 onward. A slot is free iff state=0. All data, owner, epoch
and state are physically erased on drop; only generation persists. During
start the compiler-generated code allocates the first eligible slot, bumps
the generation, initializes its contents, and evaluates the first source
segment, returning an opaque handle.

A handle is `generation*16 + slot_index + 1`. Generation is constrained to
the signed-i32 safe range and never wraps or resets. Lifecycle functions
validate handle encoding, stored generation, active state, owner and epoch
before any frame access. Reuse after drop cannot authorize an old handle.
A permanently exhausted slot fails closed instead of recycling a generation.
Do not treat numeric owner/epoch values as authentication of an untrusted
Worker; the host must bind them to a trusted owner and transport capability.

The **only source shape** admitted is the v3-proven closed-world scalar task:
1–8 straight-line literal or initialized-local `await` sites, with up to eight
`int` locals, simple assignments and an explicit final `return await`.
Maximum 32 body statements. Source control flow, arbitrary expressions,
pointer/reference/borrow live ranges, captures, multiple source-defined task
functions, `Future<T>` readiness, external I/O and error propagation remain
unsupported and return an explicit compiler diagnostic under the v4 target.
Each `await` here is an unconditional cooperative yield, not a wait on
externally pending `Future<int>`.

The full Wasm memory is fixed and never grows. A completed or cancelled slot
continues to occupy arena capacity until the owning host physically calls
`task_drop_v4`. A failure to drop is a resource leak and will eventually
exhaust the quota. Cancellation erases scalar payloads and prohibits a
subsequent resume. This profile doesn't yet implement task trees, supervision,
cross-Worker completion, preemption of malicious synchronous guest code or
contended `Mutex<T>.lock_async()`.

## Required tests

`cargo test --locked --test portable_task_v4` checks explicit compiler
admission, source determinism, v3 immutability and denied unsupported shapes.
`node tests/portable-v4-e2e.mjs` with `ATTOC=target/release/attoc`
checks actual Rust-compiled guest Wasm, 16 interleaved active task frames,
quota, cancellation, stale ownership/epochs, double-drop, physical reclamation
and repeated generation advances. Chromium/wasm32-specific and full compiler
tests must also pass on a source-exact runner before changing readiness.

Future milestones: graph-based local spilling; typed pending `Future<int>`;
authenticated completion transport integrated into task wakeups; per-task
supervisor/TaskScope ownership and contended async mutex semantics.

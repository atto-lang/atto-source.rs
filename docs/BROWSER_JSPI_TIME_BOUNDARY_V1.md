# Browser JSPI timer admission and cancellation boundary

This change is stacked on Rust browser timer preview PR #226. It is a **host
adapter**, not a JavaScript implementation of the `std/time` language module.

The compiler preview accepts `await sleep(100ms)` and
`await std.time.sleep(100ms)` via the `atto.sleep_ms(i64)` JSPI import.
`sleep_blocking` remains rejected with `E_BLOCKING_CONTEXT`. General
`Duration` values, `sleep_until`, actor timer ownership and a native
`TaskFuture<void>` ABI are not implemented by this change.

The Worker now imports `MonotonicSleepProvider`, which:

- Enforces a maximum of **64 pending timers per Worker** (and max 1000ms
  per timer, retained from the existing preview).
- Measures time once with `performance.now()` and rechecks the same deadline
  after every host wake. Spurious/early timer events rearm only the remaining
  duration, never resolve the JSPI Promise early. Nonfinite or regressed
  clock samples fail closed and release the pending timer slot.
- Enqueues even zero-delay timers to a later event-loop turn.
- Tracks pending-registration capacity and releases it exactly once on
  completion, `AbortSignal` cancellation, or scheduling failure.
- Rejects malformed, fractional, negative and out-of-budget delays before
  allocating a timer, and does not run guest code from timer callbacks.
- Does not suppress an already-running Wasm guest stack: JSPI itself owns
  suspension and reentry. The production playground creates a dedicated Worker
  per Run and terminates it on completion/timeout.

Unit evidence now includes nine cases (including regressed/nonfinite
clock readings and mid-wakeup regression). Unit evidence must be **executed** from the same helper used in the Worker:
`node --test tests/browser-async/monotonic-sleep.test.mjs`.
Full compiler-backed browser evidence additionally requires building the
native Rust compiler + wasm32 compiler and executing the real guest Wasm in
Chromium via `node tests/browser-async-e2e.mjs`; both are CI steps.

Still missing: typed `Duration` value lowering, arbitrary timer horizons,
unforgeable actor/task generation registration, explicit cancellation tree,
live actor teardown fences, and Java/LLVM/NET/Forge source-level parity.
These must stay marked unverified. This branch is stacked on #226, whereas
`atto-source.rs#545` is the independent draft `.atto` value module.

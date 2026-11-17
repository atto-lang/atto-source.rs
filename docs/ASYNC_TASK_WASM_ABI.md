# Native AttoLang tasks and suspension: browser Wasm execution contract

**Status (2026-11-12): experimental native Wasm/JSPI task slice.** Rust/Wasm compiles `suspends fnc`, class `suspends mtd`, `await`, and capture-free `task { ... }` with `Future<int>`. A real browser Worker scheduler bridges the versioned ABI. `do task` and actor-owned suspending methods remain unsupported and must fail closed.

## Surface syntax (proposal; align with core language before graduating)

```atto
suspends fnc delayedAnswer(): int {
  await rt.sleep(25);           // proposed async timer API: duration in ms
  return 42;
}
pub suspends fnc main(): int {
  val result = await delayedAnswer();
  stdio.println(result);
  return result;
}
```

```atto
pub suspends fnc main(): int {
  val a = task { return 20; };  // Future<int>, child of main task scope
  val b = task { return 22; };  // starts before either await
  val result = await a + await b;
  stdio.println(result);
  return result;
}
```

- `suspends fnc f(): T` declares a suspension effect and the **eventual result type** `T`. As a proposed source rule, invoking it in a suspending context yields a `Future<T>`; `await f()` extracts the eventual `T` (and may suspend). This is distinct from changing its declaration to `Future<T>`, and must be made consistent across backend call sites.
- An expression `task { ... }` evaluates to a **typed, single-completion `Future<T>`**, scheduled as a child of the current task scope. A `return` inside the block returns from that task only; it must not return from its lexical parent.
- `await expr` requires a callable explicitly marked `suspends fnc` or `suspends mtd`; an unmarked caller is a compile-time `E_EFFECT`. **Every executed await is a mandatory cooperative suspension**, including `await 42;` and an already-ready future. The compiler emits an `atto.cooperate() -> void` JSPI import after evaluating the operand (preserving the value on the Wasm stack), and the Worker returns to its event loop for a new task turn before continuing. A pending `Future<int>` additionally waits for its completion. Class methods may suspend; actor methods must fail closed until actor-owned carrier and non-reentrancy controls are implemented. No call executes on a supervisor control pool.
- `do task { ... }` schedules work and returns `void`, with explicit scope/supervisor ownership; it is **not** an immortal, unsupervised background job.
- Multiple task handles become runnable before awaiting them. No implicit auto-flatten, no transfer of borrowed mutable actor state or captured closures across worker/actor boundaries. A task block may suspend, and its result must not be evaluated eagerly at the spawn expression. Task closures and frames obey ownership and lifetime checks.
- Cancellation and errors propagate to the owning scope/supervisor. Each completion is exactly once; cancellation, actor generation changes and hot reload fence all stale wakeups.
- This design does not retroactively claim that Java/.NET/LLVM/Forge compilers implement this syntax. Reference grammar contains `suspends` and unary `await`; the browser subset needs an actual runtime. `task { }` syntax and the exact `rt.sleep` spelling are subject to cross-backend approval.

## Portable lowering: compiler-generated continuation state machine

For each `suspends fnc`, split basic blocks at potentially suspending calls; spill live locals into an **actor-/task-owned frame arena**, assign resume-state tags, synthesize resume/drop/cancel entrypoints, and generate ordinary Wasm functions. The runtime scheduler owns `TaskId`, generation, wake queue and task tree. Suspend returns a pending status to the worker event loop; future resolution enqueues the task for resume. **No JavaScript user-code transpilation or monkey patching.**

A proposed versioned `atto.browser.guest.v3` interface (not yet implemented) exposes a bounded `task_start/task_resume/task_cancel/task_drop` lifecycle with status `READY/PENDING/DONE/FAILED/CANCELLED` and type-safe result slots. All handles must be generation-fenced, resource-metered, non-forgeable at guest import boundaries, and fully reclaimed. A task may wake once or spuriously; the compiler must re-check readiness and keep exactly-once completion. The browser host never accepts raw code, callbacks, imported dynamic functions, arbitrary Wasm tables, or unverified imports.

The guest host can schedule over `queueMicrotask`, web timers, and approved async adapters; JS **Promises are a boundary representation**, not the Atto task ABI and not shared actor memory. Browser Workers offer tenant isolation; each worker owns its Wasm instance and mailbox. Do not move a raw stack pointer between instances. If using a 32-bit Wasm memory, frame handles still need explicit generation/owner validation.

### Choosing a Wasm async integration

1. **Portable default:** explicit compiler-generated state machines + host polling/wake operations. This works in browsers lacking JSPI; no native stack suspension support required.
2. **Optional browser optimization:** where tested and enabled, use `WebAssembly.Suspending(importedPromiseReturningFn)` and `WebAssembly.promising(exportedFn)` to bridge approved Web APIs across a Wasm stack. It solves *host I/O suspension*, **not** task ownership, cancellation, task scheduling or isolation by itself; do not assume universal JSPI support.
3. **WASI 0.3 Component Model adapter:** eventually lower `Future<T>` and `Stream<T>` to async component-model types when that target is selected. Keep source-level semantics identical to the browser and other backends.

All three strategies need the same exact-once task state machine semantics, cancellation/supervision, and borrow-checking. `await` must retain tracing/execution-context across suspension and resumption and release non-reentrant carrier resources.

## Required executable acceptance gates before changing preview to runnable

- Native Rust compiler + wasm32 compiler build; source/head/tree pin and reproducible guest Wasm byte parity; no zero-step CI.
- Positive: delay truly yields then resumes with result 42; two `task` expressions become runnable before awaits; a synchronization-barrier fixture **must deadlock under sequential/eager execution yet complete with concurrent scheduling** (without timing-only assumptions); `main()` returns 42 through **actual guest Wasm**; only chosen branch side effects run.
- Positive: `await 42;` and `return await 42;` compile and return 42 after **separate real JSPI cooperative turns**; the guest imports `atto.cooperate` and carries the version 2 `atto.async` marker. Class `suspends mtd` also yields, and non-suspending method callers are rejected. Awaiting a Future also cooperates even when it has already resolved.
- Negative: await in non-suspending callable, unsupported reference ownership across await, escaping mutable borrow, sending closure across actor, orphaned `do task`, invalid handles, double resume, double cancel, stale generation, runaway task, resource quota and timer cancellation.
- Browser: real Chromium + Worker path; host only receives a verified, strict import allowlist; no mutable cross-worker memory. Exercise timeout and stop without lingering callbacks, nested tasks and pending I/O. Verify JSPI-disabled path.
- Preserve actor non-reentrancy and suspension execution context, and prove sibling tasks survive child failure where supervisor policy allows.
- Only after these tests pass: pin a new compiler ABI, update both playground builds, change preview tests to assert real `144` or `42`, independent review, explicit public Wasm disclosure approval and protected Pages deployment.

## Current guardrail

The experimental Wasm guest uses `atto.async` version 2: versions 0/1 are no longer accepted for new execution because they do not guarantee cooperative yield at every await. Old guests must fail closed, not run with weaker semantics. Unsupported `do task`, actor suspending methods and generic futures remain explicit failures.

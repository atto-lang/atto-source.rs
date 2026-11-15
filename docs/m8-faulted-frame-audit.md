# M8 Rust/Wasm faulted-frame safety audit

**Scope:** narrow, compiler-generated Wasm actor loop (atto-source.rs #161). This remains a one-frame-per-Wasm-instance guest ABI, not full actor scheduling or generic AttoLang compiler support.

## Defect

The original compiler emitted checked integer arithmetic using Wasm `unreachable` traps. After an overflow, the frame's persisted status remained `RUNNING` (3), because the resume method had set that status before executing the loop. `m8_resume` correctly rejected further execution, but the host could not query a fault reason or call `m8_close` (which permitted only SUSPENDED/DONE). A reusable worker thus acquired a frame it could not clean up or reincarnate. This also made a permanent RUNNING status indistinguishable from an actually executing callback.

## Fix

- Add an explicit terminal `FAILED` status (4), with an independent mutable failure-code global. Before a checked arithmetic trap, the generated Wasm writes FAILED and a fault code, then emits `unreachable`. WebAssembly traps do not roll back already committed globals.
- New `m8_fault(handle): i32` guest export: 0 = no recorded failure, 1 = checked accumulator overflow, 2 = checked index increment overflow. It accepts only the currently live generation, including FAILED; a closed or stale frame traps.
- `m8_resume`, `m8_result`, `m8_pc`, and `m8_index` reject FAILED. `m8_close` accepts SUSPENDED, DONE or FAILED, **never RUNNING**, and clears all frame locals and failure reason. After close, a new `m8_new` increments the generation and initializes a fresh frame; the old handle stays invalid.
- Failed, non-mutating admission such as an invalid budget does not mark an otherwise healthy frame FAILED.
- Arithmetic preconditions are evaluated before any addition/increment writes; a checked trap does not replay a previously committed iteration or report DONE.

## Required executable acceptance evidence

The existing M8 Rust + wasm32 native-vs-browser byte-parity suite must run these cases in an actual emitted WebAssembly guest:

1. 100,000 input iterations with 4 reductions per slice, exact result 4,999,950,000; no effects replay.
2. Arithmetic overflow traps and leaves fault=1 on the same handle. FAILED cannot resume, provide a result or report a suspended PC.
3. FAILED cannot admit another frame without explicit close. Close clears the failure, invalidates the old handle; a newly created incarnation has a greater generation, fault=0 and can finish independently.
4. A 65,536-reduction slice executes at most exactly that many loop updates and YIELDs when one iteration remains. The following slice completes the same frame.
5. Closing a genuine SUSPENDED frame prevents subsequent resumption and double-close.
6. The compiler still rejects invalid borrow/critical access, `await/select`, malformed source, and illegal integer literals. The entire original Rust compiler, browser Wasm guest, Chromium workers and ABI fixture CI must remain passing.

**Proof requirements:** on source Actions quota failure, mirror the entire immutable source Git tree through the GitHub Git object connector. Verify individual changed blob SHAs and identical full root tree SHA before executing the carrier's own Actions. Reject skipped, cancelled, discovered-only or pending tests; cite real completed job and log markers. Never merge temporary carrier.

**Nonclaims:** panic-to-supervisor CONTROL dispatch remains a HOST obligation (the exported fault code is not itself a delivered supervisor notice). No native cancellation of a physically running Wasm function, full browser actor host, `await/select`, Java/Rust ABI equivalence, production tenant scheduling, hard real-time guarantee, LLVM/Forge/.NET parity or asynchronous thread preemption.

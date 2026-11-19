# Worker-local state lease ABI v1 (Rust/Wasm candidate)

This describes the admitted synchronous single-Worker profile, **not** cross-Worker shared reader/writer synchronization.

## Compiler contract

- Checked typed IR distinguishes StateRegister, StateFieldRead, StateFieldWrite and StateCriticalWrite from ordinary class operations.
- Object creation registers fully initialized state-container receivers with the Worker host before returning them.
- A protected write scope lowers to state_enter(self, WRITE) and state_leave(ticket) around pure synchronous scalar assignments.
- A protected read lowers to state_enter(self, READ), a by-value projection and state_leave(ticket) while the value remains on the Wasm stack. Reads inside an exclusive scope use the existing ticket.
- Source cannot express the host hooks, forge a continuation identity, return a lease or suspend inside the admitted critical scope.
- Host runners must reject unknown imports, rather than providing no-op admission hooks.

## Host contract

The Worker owns container IDs, actor incarnation and opaque i64 admission tickets. The guest presents only a compiler-generated allocation pointer and a mode. The host validates registration, authority, generation and release. An arbitrary untrusted Wasm module is NOT independently authenticated by this API: the browser runner's trusted compiler output policy is a separate prerequisite.

Synchronous guest imports cannot park or wait. Contention, unknown pointers, stale tickets, replay, capacity overflow and admission after retirement fail closed. On successful physical completion, the host requires zero outstanding leases. On panic/trap, the host aborts after synchronous guest execution physically unwinds and drops active leases. Cross-Worker scheduling remains unsupported.

The Rust state_admission::Gate is a separately tested bounded reference scheduler policy with waiting queues, reader cohorts, writer fairness, physical-completion lifetime pins and generation-fenced retirement; it is not yet the guest Wasm queueing implementation. Java/LLVM host-state tests are also not guest-source execution parity.

## Required progression for shared-state parity

1. Authenticate a real container and actor incarnation within a shared host scheduler.
2. Add resumable guest continuations and asynchronous admission; never park carrier or browser Worker threads.
3. Prove cancellation before/after admission, panic/raise/throw unwind, exactly-once physical completion, retirement and reclamation in executable guest fixtures.
4. Run identical compiled source fixtures across Java, LLVM and Rust/Wasm.
5. Approve and pin a browser compiler release independently from the runtime ABI.

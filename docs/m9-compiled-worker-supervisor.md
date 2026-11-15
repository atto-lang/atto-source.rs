# M9 restricted compiled Atto actor worker/supervisor bridge

The Rust compiler's existing M8 narrow `define actor` / `rt cooperate` path emits real, deterministic guest Wasm from an `.atto` file. M9 adds a separate browser/Node **Worker** executable bridge: `runtime/m9-actor-worker.mjs` accepts a validated, no-import compiled M8 guest, instantiates it in a dedicated Worker and executes its saved-PC continuation one reduction slice at a time. `runtime/m9-supervisor-host.mjs` provides bounded actor inboxes, independent priority supervisor CONTROL delivery, exact-once DATA settlement, STOP-drain, PANIC-fence, immutable host-owned actor ref authority, and generation/message-sequence checks.

### Failure delivery, not just fault storage

On a checked i64 overflow the generated Wasm marks its frame FAILED and makes `m8_fault()` report the reason. The Worker closes the now physically quiescent Wasm frame and sends FAILED to the host, **which** fences new sends, accounts the in-flight accepted message as FAILED, abandons previously queued messages, and delivers **PANIC CONTROL** to the named supervisor independently of DATA mailbox capacity. Repeated STOP is coalesced; a later PANIC is delivered in addition to an already-delivered STOP. The notice contains kind, actor_id, generation, supervisor_id, accepted message_id, numeric fault_code and short reason.

A YIELD remains one in-flight message with unchanged admission count. The host never starts a second message in that actor until the current frame DONE or FAILED. STOP fences new admission but drains accepted queued and suspended continuations. External PANIC sends CANCEL, and host does not reclaim an in-flight frame until it receives Worker CLOSED; if a Worker exits, a distinct worker-exit PANIC is generated after OS Worker exit.

Conservation on every host snapshot: `accepted == completed + failed + abandoned + queued + inFlight`. The host rejects forged actor-reference objects even with matching numbers, wrong actor/generation/message ID replies, duplicate completions and out-of-range results.

### Mandatory real execution

- Run the native Rust compiler and wasm32 compiler, then compile actual `tests/fixtures/m8/cooperative_sum.atto` to executable M8 guest Wasm.
- Execute in independent Node `worker_threads` workers: healthy actor executes >10k compiled loop iterations with multiple continuations, STOP drains; faulty actor has a saturated bounded DATA inbox when checked overflow occurs, yields one PANIC notice and accounts every accepted message.
- Execute the same M9 worker host and compiled guest in **real Chromium browser Workers** as part of `tests/browser-e2e.mjs`, with failure notice/STOP and exact settlement.
- Assert host PANIC cancellation of a genuinely suspended continuation does not resume or reclaim before Worker CLOSED, and no forged or stale actor reference is authorized.
- Run all existing Rust source/guest/Chromium tests, in addition to the specific M9 workflow, on SHA-identical private Actions carrier when source-org CI cannot execute.
- Verify every changed Git blob and full exact Git tree SHA; never merge the carrier, and never use secrets or cross-org private source checkout.

### Explicit limitations

This is a **restricted dedicated Worker per actor** host for the M8 signed-i64 accumulator loop. It is NOT the regular `atto.browser.guest.v2` actor-message runtime, nor a shared bounded worker pool, full language `receive()`, language `await/select`, a general borrow checker, enforced CPU preemption, real-time certification, or LLVM/Forge/.NET backend parity. A crashing or malicious Worker is outside the guest's memory and may be terminated; cross-worker supervisor delivery at the host is not by itself a formal multiprocess ordering proof.
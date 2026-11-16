# Browser checked write-admission runtime (experimental)

**Status: host-side executable component, not yet an AttoLang guest API.**
The source compiler continues to reject `nb writech`, `nb source << value`, `do writech`, and `do source << value` until a fully checked typed `Future<Result<...>>` guest ABI and compiler lowering exist. In particular, the current JSPI `Future<int>` task handle must **not** be reused as an I/O outcome: it cannot represent EOF, close, cancellation or delivery failure without losing error typing.

`runtime/browser/checked-write-queue.mjs` provides a bounded, single-event-loop-domain **asynchronous host write adapter** that can be integrated behind a future channel import. It is used directly in the Node acceptance tests and inside a real Chromium Worker; it does not pretend to execute an Atto guest `nb writech`.

## Contract

- `new CheckedWriteQueue({slots, maxBytes, sink, supervisor})`; both quotas are finite and enforced on queued **plus committed** writes. `sink(bytes, ticket_metadata)` is a nonparking async adapter that must acknowledge exactly `bytes.length` or return a structured error. No OS flush or peer delivery guarantee is inferred from admission.
- `submitWrite({owner, generation, payload, mode:'nb'})` returns `{ok:true,ticket,future}` on acceptance; `future` resolves to `{ok:true,written}` or `{ok:false,error:{code,message}}`. The returned Promise is an *internal host future*, **not** the guest `Future<Result>` ABI.
- `submitWrite({owner,generation,payload,mode:'do',onOutcome?/onSuccess?})` requires a supervisor error channel, returns `{ok:true,ticket}` (no caller future), and reports terminal outcomes once. `onOutcome` is typed success/error and `onSuccess` runs only upon successful completion. No success callback can absorb a failure. Suspended/asynchronous callbacks are forbidden and reported.
- Admission rejection returns the **original payload** synchronously with an error; the caller retains ownership. Successful admission makes a bounded independent byte copy to prevent caller mutation; the future compiler lowering must still enforce move/borrow rules.
- A queue is FIFO and processes one committed sink call at a time. `cancel(ticket)` succeeds only *before* sink commit; a committed sink call cannot be rolled back, and `cancel` reports false. `stopOwner(owner,generation)` fences callbacks and cancels that owner's queued work; committed work must finish according to its adapter contract. All owner/error events retain their generation identity.
- `close()` fences new submissions and cancels queued work, while committed writes finish; credits and retained byte quota are released exactly once. Cancelled tickets are removed immediately, preventing an unbounded tombstone FIFO under repeated cancel/re-admit.
- Supervisor exceptions remain observable through `takeSupervisorFailures()` (the Worker controller must inspect these before teardown); they must not be interpreted as completed detached operation delivery.

## Evidence / limits

`node --test tests/browser-checked-write-queue.test.mjs`: asynchronous host tests for FIFO, capacity and UTF-8 quotas, early rejection preserving payload, copy-on-submit, write failure, success/error callbacks, cancellation-before/after-commit, owner restart, close, callback errors, and supervisor escalation.

`tests/browser-async-e2e.mjs` exercises this host runtime inside the actual Chromium Worker via a separate `probe: 'checked_writer'` message. It also retains existing Rust-compiled Wasm guest `await`/task tests and verifies `nb`/`do` guest syntax still fails closed. A green Worker host probe is **not** guest I/O execution, and there is no claim of native, Java, LLVM, .NET, Forge or cross-backend parity.

Remaining: add a nominal typed writable capability, actual `Future<Result<WriteAck, IoError>>` source type and Wasm ABI, import allowlist and typed lowering, move/borrow checks, checked backpressure policy, supervisor CONTROL integration, real guest fixture proving in-order callbacks and cancellation, and cross-backend conformance. Keep the PR draft until those milestones exist.

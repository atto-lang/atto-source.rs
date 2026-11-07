# AttoLang browser actor lifecycle — Stage 3 private subset

**Status:** experimental, source-owned implementation only; not public release
authorization. Independent review and exact-head CI are mandatory.

## Language boundary

The existing stage-2 send(int) returns Result<void, SendError> for
**mailbox admission only**. It never proves that the handler ran. Stage 3
additionally admits only the terminal await actor.get_done() statement as the
final direct statement of a void main():

    define actor Worker as
      receive(msg: int) -> int { return msg + 1; }
    end

    pub fnc main(): void {
      val worker = spawn Worker();
      val accepted: Result<void, SendError> = worker.send(41);
      await worker.get_done();
    }

The compiler emits the atto.actor_join(i32 handle) -> void import to register
the terminal structured join. This import does **not** block Wasm or claim
completion. The guest returns from its only admitted await position, then
the browser host drains the admitted queue, explicitly closes the actor,
and suspends using a JavaScript Promise until the child Worker sends done.
The browser result is reported only after verified terminal acknowledgment.

Arbitrary await expressions, statements after the terminal join, nested
awaits, non-void main joins, get_done as an unawaited task value and
non-actor joins fail closed without producing guest bytes. The compiler
does not yet preserve general suspended Wasm frames, continuation locals,
Task<T> or full Result<T,E> generic types. No general async/await claim.

## Host lifecycle state machine

Each actor owns a dedicated Worker and Wasm instance. Its host task exposes
a genuine asynchronous get_done() Promise with one terminal resolution.
States: starting -> ready -> draining -> closing -> completed, or failed /
cancelled.

- admit(i64) linearizes into a host-owned FIFO queue (32 messages maximum)
  before any child delivery; status 0=Ok, otherwise Err(closed/full/invalid).
  Object/class/pointer transport remains forbidden.
- drain_and_close() closes admission, waits for child readiness, sends each
  queued message and waits for its reply before sending the next.
- The parent then sends {kind:close} and waits for the child's terminal
  {kind:done,processed:N}. Both sides must agree on N. Worker termination or
  postMessage alone never resolves the completion task.
- Receive traps report E_ACTOR_PANIC_<code> where available. Worker/protocol/
  clone errors, missing done, stalled handler, or cancellation reject the task.
  Timeouts and cancellation terminate the child. Late messages cannot
  resurrect an already terminal task or convert failure to success.
- Actor completion is scoped to one browser program invocation: all spawned
  workers are drained/closed before reporting successful main completion.
  Terminal source await explicitly registers a join for its actor.
- actor_join and actor_send cannot execute inside actor receive handlers.
  Compiler transitive host-effect checks remain active for helper call graphs.

## Evidence requirements and deferred semantics

Source-owned CI executes native Rust, builds the real wasm32 compiler, runs
native-vs-Wasm conformance, Node lifecycle/race tests and Chromium source to
compiler Wasm to guest Wasm to dedicated actor Worker scenarios. Tests must
prove terminal join succeeds only after child acknowledgement, and rejects on
actor panic, cancellation, timeout, stale acknowledgments, or protocol faults.

**Not implemented:** supervisor priority CONTROL path, actor-owned arena
reclamation, separate Wasm stacks, general task suspension, resource capability
revocation, true cross-tenant OS sandboxing, production scheduling, Java/LLVM
or five-backend parity, public playground artifact approval. A Web Worker is
not an OS security sandbox.

Do not transfer source/compiled artifacts or publish to Pages without separate
independent source/disclosure approval and public Pages protections, as
recorded in https://github.com/atto-lang/atto-lang.github.io/issues/5 .

## ABI version boundary: browser integrated.v1 vs experimental scalar.v0

The separate exploratory PR #19 exposes `compile_actor` and standalone,
import-free actor guest bytecode with only `receive(i64)->i64`. This is **not**
the integrated browser actor ABI in this branch, and its `compile_actor`
entry point is **not** silently included in the public browser compiler.

The browser IDE uses the integrated compiler contract: `main` plus
`atto_actor_receive_<id>` handler exports, with controlled actor imports for
source `spawn`, `send`, and terminal `get_done`. The standalone `receive`
export is explicitly rejected with `E_ACTOR_ABI_MISMATCH` **before actor
worker execution**, not aliased or guessed. The staged public host contract
at `cf9fec5f6957fc5b1c53e204b2c39b5e84af5b35` contains a valid standalone-v0 Wasm negative fixture, plus
successful integrated-v1 actual-worker tests. Do not combine these ABIs by
copying PR #19's `compile_actor` into the integrated source/compiler without
an independently reviewed versioned API and full cross-mode negative tests.

This branch does not publish private compiler Wasm to GitHub Pages. Release
requires source provenance, a reviewed exact compiler digest, independent
review, and the Pages protections in atto-lang/atto-lang.github.io issue #5.

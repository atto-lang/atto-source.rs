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

## Stage 3+ source-owned protocol hardening

Each accepted parent-to-actor message carries a zero-based monotonically
increasing integer sequence number. The dedicated child Worker checks that
the sequence matches its completed-message count before executing receive().
Every reply echoes this exact sequence number; the parent verifies that it
matches its current pending delivery. Duplicate, future, malformed, missing
or replayed response IDs therefore **fail closed** instead of acknowledging
another queued message. Signed i64 payloads remain the only admitted
transfers; the sequence number is protocol metadata, not user data.

Fault codes received from a worker must be a bounded (up to 128-character)
ASCII E_ code; arbitrary user-controlled error strings are mapped to a
fixed E_ACTOR_FAULT code. The completion task never resolves on a reply,
successful admission, or termination; only close plus the checked child
done(processed) event can complete it.

Expanded verification includes constructor and transport validation, i64
extrema, zero-message close, duplicate drain and ready messages, replayed,
out-of-order and malformed reply sequence IDs, cancellation during delivery,
missing/incorrect terminal acknowledgements, protocol faults, synthetic
clone faults, independent multi-worker lifecycle outcomes and a deterministic
seeded 32-turn FIFO stress model. Browser tests execute actual compiled
Atto source using nested Web Workers for both i64 limits, zero-message
completion, multiple actor joins, overflow after an earlier valid reply,
mailbox capacity at terminal join and post-await rejection/recovery.

These checks **do not** certify a general async/await state machine or
cross-tenant protection; the supported source suspension remains terminal
structured join only, and the worker is not an OS security sandbox.

## Stage 3++ cancellation and import-boundary audit

Cancellation is an immutable terminal outcome. A resolved ready Promise
can race with synchronous cancellation before the awaiting drain resumes.
Likewise, a reply Promise can resolve immediately before cancellation, or
the on_reply observer can cancel the actor. A resumed continuation must
check the actor's terminal state *before changing it* and before invoking
reply observers, sending another message, or issuing close. The original
terminal Error object remains stable for all subsequent observers. Failing
to recheck would allow a cancelled actor to become draining/closing again
and could permit an invalid done acknowledgement to change observable state.

A single source-owned strict guest-import validator now runs in both the
compiler worker and the child actor worker. Import entries must use the exact module name atto, one of the seven
explicit capability names, kind function, and at most one occurrence per
name (no alias slots). More than seven imports is rejected. The subsequent
Stage 3+++ gate also validates all declared function signatures from the
exact source bytes; compiler-produced guests remain provenance-pinned.

Regression gates cover cancellation between a ready resolution and its
scheduled drain microtask, between a reply resolution and the awaiting
observer, observer-initiated cancellation, multimesage cancellation,
first-error preservation and 128 deterministic ready/cancel interleavings.
Node tests construct actual valid Wasm modules containing duplicate,
foreign, global, memory and table capability imports; Chromium tests inject
malformed Wasm import modules into a real child worker and require rejection
before actor instantiation. These tests are not full tenant-isolation proof.

## Stage 3+++ exact Wasm ABI signature gate

The parent compiler Worker and child actor Worker now validate the **raw
Wasm guest bytes** against the exact canonical Atto host import function
signatures, in addition to the existing module/name/kind/uniqueness gates.
A caller cannot validate a different Module object's import metadata while
instantiating another binary. The validator first calls the native browser
WebAssembly decoder and then reads the bounded core Wasm type/import
sections, resolving each function import's type index. It accepts
custom sections without treating their contents as core import metadata.
The supported scalar profile allows only i32 and i64 function types.

Exact type contracts (function parameters -> result):

- print_i64: (i64) -> void
- print_bool: (i32) -> void
- print_str: (i32, i32) -> void
- print_newline: () -> void
- actor_spawn: (i32) -> i32
- actor_send: (i32, i64) -> i32
- actor_join: (i32) -> void

A mismatched function type fails closed with E_ACTOR_IMPORT_SIGNATURE
before any guest instantiation, including when a function has an allowed
name but different stack argument/result types. Node tests construct
real valid Wasm binaries for all seven positive types, altered arguments,
altered result types, custom sections, malicious typed aliases, and raw
buffer offset views. Real Chromium tests instantiate dedicated actor
Workers with deliberately wrong scalar ABI imports and check rejection.

This establishes **the declared import function ABI**, not full sandboxing:
memory limits, arbitrary Wasm exports, dynamic resource capabilities,
untrusted host JavaScript, tenant process isolation, general async
suspension and per-actor heap/stack enforcement remain separate gates.

## Stage 4A — host-owned supervisor terminal CONTROL path (browser subset)

The browser parent owns a separate SupervisorControl instance with four
reserved actor slots, matching the maximum four actor Workers per guest run.
Each ActorSession registers an opaque parent-owned capability token before
worker initialization. The token is never sent to child Wasm or to a nested
Web Worker. Child messages cannot author a terminal CONTROL event: only
the parent's own verified lifecycle state transition records one.

A terminal notice is an immutable record with:
- route=CONTROL and kind=terminated | panic | failure | cancelled | timeout
- state=completed | failed | cancelled
- actor_handle, actor_id (source actor type ID), parent_id
- bounded error code (or OK), processed-message count

The first terminal event is permanently journaled for its actor handle,
and subsequently delivered at most once via take_control, drain_control, or
wait_control. The priority take_next API consumes queued CONTROL notices
before any ordinary actor messages. CONTROL slots are independent of the
actor's 32-message FIFO, so normal mailbox saturation cannot drop a terminal
event. Four actor sessions each reserve a slot before their worker starts;
a fifth session fails closed. The journal is retained even after dequeue.

The parent publishes terminal records before resolving/rejecting its
get_done Promise. A genuine close/processed ACK yields terminated;
a runtime panic yielded by the trusted child execution wrapper yields
panic; explicit cancellation yields cancelled; timeout and other host
failure events retain distinct kinds. Late/duplicate child replies,
post-termination errors and repeated cancellation cannot rewrite the
journal or create duplicate terminal notices. This is trusted-host
capability gating, not cryptographic child authentication.

The browser compiler Worker exposes a control array in its test-facing
postMessage result. The playground UI continues to display the existing
output; no new public release is authorized. Integration tests execute
actual source-compiled actor Wasm in nested Web Workers and inspect host
CONTROL notices for multiple normal terminations, handler panic, main
panic/cancellation, and full-mailbox shutdown. Deterministic Node
tests cover priority delivery, spoofed child CONTROL messages,
fault classification, four reserved slots, and races.

**Scope limit:** these terminal notices are host-private and run-scoped,
not yet a general language-level Supervisor actor API or a distinct OS
priority queue. Java/LLVM/Forge/.NET parity, multi-tenant hard isolation,
actor-private allocator domains, general Task/await continuations, and
fully preemptive watchdog enforcement remain unimplemented.

## Stage 4B — typed source-level supervisor watch admission (narrow browser subset)

The compiler now admits **empty nominal supervisor declarations** and typed,
host-mediated supervisor creation and watch admission:

```text
define supervisor Monitor as end
define actor Worker as receive(msg: int) -> int { return msg + 1; } end

pub fnc main(): void {
    val monitor: Monitor = spawn Monitor();
    val child: Worker = spawn Worker();
    val receipt: Result<void, SupervisionError> = monitor.watch(child);
    val admitted: Result<void, SendError> = child.send(41);
    await child.get_done();
}
```

- `spawn Monitor()` creates a **host-owned** supervisor handle, distinct from
  a normal actor handle. It does **not** create a separate nested Worker, an
  actor message mailbox, or a Wasm guest allocator.
- `monitor.watch(child)` requires a compiler-typed supervisor and a
  compiler-typed actor. It returns only the specialized
  `Result<void, SupervisionError>` admission result (0 = Ok, nonzero =
  Err); `is_ok()` and `is_err()` are supported but a successful watch does
  not mean a child has executed or terminated.
- Two new exact function imports, `atto.supervisor_create(i32)->i32`
  and `atto.supervisor_watch(i32,i32)->i32`, are checked against the
  raw Wasm binary's canonical type/import sections in the **same** strict
  ABI validator as existing actor imports. Invalid kinds, signatures,
  aliases, duplicates, modules and unsafe nested effects fail closed.
- Watch registration changes only the **parent_id** of future host-owned
  CONTROL terminal records. An unregistered child stays with the run-level
  default parent. Each child may have at most one watched parent, forever;
  duplicate watching returns Err, and terminal actors cannot be adopted.
  The parent attribution is committed before terminal success/failure.
  As soon as structured drain begins, host watch admission is sealed.
  A late attempt returns Err even if no terminal acknowledgement has arrived,
  preventing mid-flight reparenting after message execution begins. Sealing
  requires the parent-only actor-registration capability.
- There are at most **four supervisor instances** and **four actor instances**
  per run. Actor mailboxes remain bounded at 32 pending messages each.
  Watch registration never consumes an actor data-message slot. Host
  supervisor handles cannot be used as pointers or message payloads.
- Child actors are **never** supplied the supervisor imports as effective
  capabilities: the child Worker binds both imports to fail-closed traps.
  The type checker also rejects direct and transitive supervisor host
  effects from a scalar actor `receive()` handler.

**Deferred behavior:** Source-level `on_child_terminated` callbacks, dynamically
implemented `extends Supervisor` actor inheritance, supervisor strategies or restart
policies, rich `ActorExit` typed values, arbitrary `Task<T>` suspension, arbitrary
continuations after `await`, independently preempted actor execution, dynamic
revocation, cross-tenant security isolation and five-backend parity are **not
claimed**. This milestone is *typed source-to-runtime watch admission* and
checked routing of the existing trusted browser host's CONTROL journal,
not full language-level supervision.

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

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

## Browser worker protocol v1 sequence fence — exact candidate pin

The source-compiler-to-public-IDE test now pins the reviewed public host
`823b6b10400597e2a9e52221d7c20872f1cf16de` (PR #21), where actor workers enforce protocol version 1,
monotonically increasing per-actor message sequence numbers, actor-ID matching
on readiness/reply/terminal ACK, and fail-closed cancellation during the
readiness continuation and reply callbacks. The public host itself has real
Chromium replay rejection and Node fault-injection receipts. The candidate
compiler must be tested against this exact frontend snapshot without copying
any private source/binary into a public branch or triggering Pages deployment.

## Exact four-actor stress-tested browser host

The private-source Chromium integration suite now pins public source-only
playground host commit `133bb2d1479f7fec5aa0615ff85fcd0f07ebb4cb`, which passed 59 Node tests
(including 120 deterministic lifecycle fault trials and 40 simultaneous
four-child cancellations) and 30 real Chromium E2E scenarios. Executable
source-free Wasm fixture verifies 4 x 32 admitted FIFO actor messages,
4 rejected 33rd sends, separate actor-local mutable state, a rejected 5th
spawn, and a one-shot parent runner. These are host and bounded runtime
receipts; this private compiler candidate remains unpublished and requires
independent approval before any public release.

## Parent-owned actor scope: exact public frontend

The compiler-in-public-IDE candidate is now tested against public frontend
PR #21 exact `802bbcbcfce1ddee0db80b5bde3494ba606048a2` with parent-owned scope, concurrent child draining,
deterministic stdout, fail-fast cancellation of all still-live siblings,
one-shot runner admission and typed/memory-bounded guest Wasm admission.
This public frontend passed 64 Node tests and 31 real Chromium scenarios.
The source checkout remains immutable and is **private CI rehearsal only**:
no copying of private compiler artifacts into public GitHub Pages.

## Exact public release-governance candidate pin

The private compiler candidate suite now checks out the exact **latest**
public host PR #21 head `36f82281adf29a08eef696754a83ba66425bab04` to include its
fail-closed `workflow_dispatch` governance preflight, GitHub Pages
administrator checklist, CODEOWNERS, and the previously tested
parent-owned actor runtime. Public PR smoke/release verification is
source-only and does not activate the protected deployment job.
Do not treat the guarded workflow or CI success as live publication.

## Final read-only governance token fix and compiler candidate pin

The public frontend reference is now exact `fbaf1e7cc6a974cac659aea8ac0e732c3dc5bc86`, including
read-only `actions:read` and `pages:read` GitHub API scopes for release
verification and source-only regression tests. Public host PR #21 at
that head passed 64 Node tests, 31 actual-browser tests and 14 guarded
release-package tests (release workflow deploy job was skipped).
CI checks class, enum, and per-actor Web Worker execution against this exact
host without publicly publishing the candidate compiler or setting approval.

## Strict Wasm-start admission and unfiltered required checks

The actual browser compiler candidate suite pins public playground source
`335ed06a0963a445a568ae86e20e9e6bad354a81`, where both runtime domains reject valid Wasm modules
containing start functions, the child has the same bounded bytecode admission
as its parent, and parent cancel-before-init is terminal. A source-backed
temporary browser fixture includes `src/wasm-admission.js` in its explicit
nonpublic asset allowlist and asserts a Rust-compiled `new Point()` guest
passes the new host's admission contract. The public host passed 69 Node
tests, 33 real Chromium scenarios and 15 release-policy tests, plus staged
browser verification. These are private CI integration checks and are not
authorization to disclose compiler bytes or deploy Pages.

## Bounded per-instance memory and noncooperative cancellation host

The source CI now pins the immutable public host `fb4a8032ef7eb3d3325222ed631c1b7db5cf3c9c` with
strict bounded guest Wasm memory admission: at most 16 x 64-KiB pages,
no unbounded/shared/imported memory or imported/local tables, and 1024
function maximum for admitted body and imports. Public host Node tests
(72) and real Chromium tests (35) include a noncooperative nested actor
infinite loop canceled by the parent. The private compiler's real
`new Point()` guest is now independently instantiated and checked to
export a fixed 64-KiB non-growable memory; the already existing
`admitGuestWasm` check also rejects disallowed memory structure.
This proves only the narrow compiler-backed guest subset, not general
AttoLang actor heaps, native stack or CPU budgets.

## Forced parent lifetime / browser CPU deadline cross-repo pin

The private compiler integration now pins public host ced47338040427e824728fc96ffeb5906982efe6. Its direct Chromium suite tests four nested noncooperative actor launches, forced parent termination, absence of late success, fresh independent recovery and an actual 450-ms wall-clock deadline. Node has 160 deterministic worker-task late-reply/cancellation/timeout race trials. Chromium target enumeration did not independently prove orphan-worker reclamation. The compiler remains privately staged; no public release is authorized.

## Page-owned supervisor and 110-run reclamation stress

Private compiler CI pins the exact public host `053dd8bff5dedcb6d41562f1198e526db0acc73e`. Its public source now owns actor worker handles on the page, transfers MessagePorts to the parent compiler runner and revokes child handles independently on parent Stop, timeout, failure or success. Public CI verifies 110 real four-actor cancellation trials (440 child handles, zero active at every boundary), 128 deterministic four-child fault trials and a fresh recovery execution. The private candidate includes both `src/actor-worker-supervisor.js` and `src/actor-worker-port.js` in its staged public IDE, and must prove actual Rust-compiled class/enum/actor execution against them. Browser Worker handle revocation is independently observed; OS process RSS reclamation remains unverified.

## Allocation, transfer, and supervisor quarantine fault matrix

The private compiler candidate now pins the exact public frontend
`2b66373c373e39c2a214dc31db05743409dbd28f`. At that head the browser runtime counts and closes children
after Worker/MessageChannel partial constructor failures, rejects duplicate
port transfers, catches timer/event-handler registration errors and refuses
actor supervisor reuse if any child Worker termination throws. The public
test matrix includes 256 deterministic four-actor terminal races, nine
real-browser child/channel/port-transfer negative scenarios, and 110
independent four-child Stop trials. The Rust compiler is exercised only in a
private ephemeral browser candidate; no private binary is published.

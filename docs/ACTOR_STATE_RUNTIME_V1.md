# Actor guarded state: physical scoped execution (experimental)

This module composes the canonical FIFO logical admission prototype
(\`actor_state_guard_v1\`) with an actual process-local \`RwLock<T>\`.
It is a **trusted-host reference**, not an AttoLang compiled guest runtime
or an authorization mechanism for user-supplied actor IDs.

A runtime creates \`Arc<GuardedActorRegion<T>>\` and stores state **only** there.
An independently authenticated actor/region capability is needed before the
host supplies a numeric actor ID to \`request\`. The host admits FIFO scopes
with \`admit_ready\` and executes a closure with \`execute_read\` or
\`execute_write\`. A scope is not Clone, not Sync, and is consumed by
execution. Callbacks get a Rust reference only while the real RwLock guard
is held, and cannot return a borrowed reference from safe Rust.

- Reads are physically concurrent; writes are physically exclusive.
- Waiting writer blocks later reader admission. Mutex/RwLock acquisition is
  **try-only** on the execution path; a busy lock yields \`RuntimeError::Busy\`
  for the host to reschedule rather than park a reusable carrier.
- A missing/foreign, stale, retired or read-to-write escalated scope fails
  before running the callback.
- Scope guard Drop sends a completed permit to a host-only completion
  channel **after** the RwLock guard is gone, including panic unwinding.
  This never waits for the queue mutex. The next admission/status/STOP call
  drains completed permits under the mutex and preserves reclamation fencing.
  A poisoned queue fails closed.
- The completion channel is unbounded by itself, but only 1,024 permits can
  exist in the admission ledger. The host must service the queue; dropping
  a scope does not automatically wake an actor. Poisoned state fails closed.
- STOP/hot-upgrade can retire an active region; it rejects later access but
  cannot reclaim until all already-entered physical scopes finish. The
  \`may_reclaim\` flag is necessary but insufficient for freeing backing
  memory, and existing \`Arc\` references keep the allocation alive.
- The queue is bounded and nonreentrant; waiting cancellation cannot revoke
  an active memory scope.

**Non-goals and incomplete boundaries:** No production scheduler integration,
no guest \`state {}\` code lowering, no per-tenant capabilities, no immutable
publication, no physical actor heap isolation, no independent proof of FFI
non-escape, and no automatic cancellation of a hung guest callback. A callback
that loops indefinitely can hold a physical read/write lock indefinitely.
Actual actor runtime integration must insert cooperative checkpoints and
isolation/timeouts, bind supervisor epochs to region identity, implement
quarantine on failed physical quiescence, and prohibit await/select/actor hop
inside guarded scopes. This process-local reference must not be installed as
a production security boundary on its own.

## Evidence

\`tests/actor_state_runtime_v1.rs\` exercises parallel physical readers,
writer ordering, failed escalation, cross-region scope rejection, STOP during
an executing read, stale scope non-execution, cancellation, panic unwinding,
and the five-mail Counter accumulation. These are **real Rust threads and
memory locks**, not compiled AttoLang guests or backend parity evidence.

## Concurrency audit (2026-11-11)

An audit found the earlier scope Drop performed a potentially blocking queue
mutex acquisition, contradicting nonblocking carrier semantics. The fixed
implementation defers permit completion; unit tests intentionally hold the
queue mutex while another thread drops a scope, including a retired region.
This eliminates *queue-mutex blocking in Drop*, not the need for host polling,
quota enforcement, actor authentication or a real tenant runtime adapter.

The reference cannot prove that `T` lacks interior-shared `Arc` aliases
or that callbacks cannot interact with FFI/raw pointers. Production code must
use a layout-certified actor state type and enforce non-escaping borrows before
exposing a state callback to guest code.

## Fail-closed completion corruption

The completion receiver treats any unexpected unknown, duplicate, or invalid
logical permit settlement as a permanent queue quarantine. Every subsequent
request, admission, STOP, and `may_reclaim` query returns
`RuntimeError::Poisoned`. A consumed malformed completion must never turn
a one-time error into a later false proof that the region is reclaimable.
The unit regression intentionally simulates an out-of-band host settlement,
then verifies persistent rejection. Do not clear quarantine without an
independent supervisor reconciliation and physical quiescence proof.

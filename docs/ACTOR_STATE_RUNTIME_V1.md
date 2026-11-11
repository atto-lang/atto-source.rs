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
- Scope \`Drop\` releases the logical permit *after* the RwLock guard is gone,
  including panic unwinding. Poisoned state fails closed.
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

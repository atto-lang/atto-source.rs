# TaskScope and Mutex cancellation bridge (restricted reference v1)

**Status: HOST-ONLY. NOT compiled guest, production scheduler, physical-retirement proof, or TenantVM certification.**

The private TaskScopeLockCoordinator in src/mutex_wait_queue_v1.rs ties a logical task identity and its lexical parent to the existing bounded mutex waiter arbiter and TaskLeaseStack. src/lib.rs still enables this model ONLY in Rust unit-test builds. This is an implementation checkpoint, not permission to enable guest task/Mutex compilation.

## Contract implemented in this reference model

- Register parent/child task identities in one authenticated tenant, actor and incarnation domain. Bound total tasks (1024), locks (256), and live permits per task (256).
- Track waiting tickets, granted-but-unclaimed tickets and actually claimed guards independently of the carrier OS thread.
- Request cancellation recursively down the task subtree, fence new children and lock claims, remove queued waiters, and revoke unclaimed grants without ever manufacturing a guard.
- Never release an active critical lease at cancel-request time. Release all held guards in reverse acquisition order ONLY after the trusted guest-safe-point exit event, on normal return or cancellation.
- Treat guest exit, callback drain and physical retirement as distinct. Parent retirement requires all children physically retired, zero pending/unclaimed/held permits and no cleanup failure.
- On failed cleanup, retain unresolved lease ownership and deny safe retirement. A cancelled task cannot certify its own physical retirement.
- Keep a parent-held guard's release ahead of waiting for child retirement; this avoids joining a child that is waiting for that same mutex.
- Reject wrong tenant, actor/incarnation, forged task segment, revoked or duplicate lock grant.

## Seven new task-scope race tests

1. Cancelled parent and active-guard child cannot retire early or hand off until safe release.
2. Cancellation of a queued waiter leaves the active holder's guard untouched.
3. Cancellation after grant but before claim invalidates stale wakeup and hands off exactly once.
4. Repeated cancellation with nested guards produces LIFO one-time release.
5. Missing cleanup dependency blocks retirement while independent guards are attempted.
6. Parent releases its guard before joining a cancelled child that was waiting for it.
7. A cancelled parent cannot spawn more children; forged tenant/segment identities fail closed.

There is also an added compiler test in tests/mutex_fast_guest_profile.rs: guest programs combining Task, Future.cancel and Mutex must keep failing closed until scheduler and lifetime proofs exist.

## Verify on exact source head

    cargo test --locked --lib task_scope_ -- --nocapture
    cargo test --locked --test mutex_fast_guest_profile typed_task_cancel_with_mutex_remains_fail_closed
    cargo test --locked
    cargo clippy --locked --all-targets -- -D warnings

## P0 production integration still required

- Replace test-only guest exit and callback acknowledgements with authenticated scheduler-derived physical receipts. Never accept guest-settable Task DONE.
- Integrate typed TaskSpawn, TaskCancelRequest, TaskJoin, LockWait, LockClaim, GuardRelease, TaskPhysicalRetire operations and verified borrow/permission effects into actual guest MIR.
- Implement versioned async guest/host ABI, wake/cancel revalidation, nonblocking carrier scheduling and bounded cancellation safe points without changing the meaning of legacy atto.async/2.
- Integrate RwLock and actor critical-state leases, callbacks, timeouts, untrusted TenantVM containment and host-side generation fencing.
- Compile and EXECUTE exact source-pinned cancellation/lock guests and nine lifecycle probes in atto-platform-conformance PR #115, with actual physical-retirement and cleanup receipts. Do not move NOT_EXECUTED backends to PASS on the strength of this reference model.

References: atto-spec PRs #125/#129, atto-source.rs issue #428 and PR #416, atto-platform-conformance PR #115.

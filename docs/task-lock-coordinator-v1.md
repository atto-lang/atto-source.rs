# TaskScope and Mutex cancellation bridge (restricted reference v1)

**Status: PRIVATE NATIVE HOST KERNEL (code authored, tests not yet executed). NOT a compiled AttoLang guest, active production scheduler integration, physical-retirement proof, or TenantVM certification.**

The private TaskScopeLockCoordinator in src/mutex_wait_queue_v1.rs ties a logical task identity and its lexical parent to the existing bounded mutex waiter arbiter and TaskLeaseStack. src/lib.rs compiles the private kernel for the native Rust host, but intentionally excludes it from wasm32 browser-compiler builds. No untrusted guest import/call path is wired, and no guest task/Mutex execution is enabled. This is an implementation checkpoint, not permission to enable guest task/Mutex compilation.

## Contract implemented in this reference model

- Register parent/child task identities in one authenticated tenant, actor and incarnation domain. Bound total tasks (1024), locks (256), and live permits per task (256).
- Track waiting tickets, granted-but-unclaimed tickets and actually claimed guards independently of the carrier OS thread.
- Request cancellation recursively down the task subtree, fence new children and lock claims, remove queued waiters, and revoke unclaimed grants without ever manufacturing a guard.
- Never release an active critical lease at cancel-request time. Release all held guards in reverse acquisition order ONLY after the trusted guest-safe-point exit event, on normal return or cancellation.
- Treat guest exit, callback drain and physical retirement as distinct. Parent retirement requires all children physically retired, zero pending/unclaimed/held permits and no cleanup failure.
- On failed cleanup, retain unresolved lease ownership and deny safe retirement. A cancelled task cannot certify its own physical retirement.
- Keep a parent-held guard's release ahead of waiting for child retirement; this avoids joining a child that is waiting for that same mutex.
- Reject wrong tenant, actor/incarnation, forged task segment, revoked or duplicate lock grant.

## TaskScope and host-runtime regression coverage

1. Cancelled parent and active-guard child cannot retire early or hand off until safe release.
2. Cancellation of a queued waiter leaves the active holder's guard untouched.
3. Cancellation after grant but before claim invalidates stale wakeup and hands off exactly once.
4. Repeated cancellation with nested guards produces LIFO one-time release.
5. Missing cleanup dependency blocks retirement while independent guards are attempted.
6. Parent releases its guard before joining a cancelled child that was waiting for it.
7. A cancelled parent cannot spawn more children; forged tenant/segment identities fail closed.

The module also contains an internal HostTaskLockKernel, which serializes trusted metadata with try_lock (returns AdmissionBusy instead of parking a scheduler carrier), uses host-issued task/lock IDs, and reserves unique generation epochs for each mutex. This prevents cross-mutex replay of identical task/sequence tickets. Trusted host STOP requests cancel all tasks and waiters before closing queues; claimed guards remain held until safe-point cleanup. Further regression tests cover nonparking arbiter contention, cancellation of a child while preserving an unrelated sibling, task/scope epoch separation, cancellation-versus-claim, tenant STOP, cross-mutex ticket replay, and duplicate-generation denial.

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

## Latest CI runner admission blocker (2026-11-20)

- The original source organization and temporary private [usa-acc carrier #6](https://github.com/usa-acc/atto-ci/pull/6) failed before executing steps. That carrier mirrors an **earlier exact source blob**; it is not a test receipt for the latest bridge.
- A new private [scintilla-run admission probe PR #6](https://github.com/scintilla-run/atto-ci/pull/6), [run #44681240718](https://github.com/scintilla-run/atto-ci/actions/runs/44681240718), also failed with zero executed steps. No source was copied to this second carrier.
- Recent Actions checks in several additional Atto private carriers likewise recorded no executed steps. Do not treat a workflow's failure status as a compiler/test failure, or any authored test as a pass.
- Next verification must execute cargo test and clippy on the exact source head and run real compiler-to-Wasm guest tests before guest support or parity claims.

## 2026-11-20 additional hardening

- The native-only private host kernel now includes a nonparking `HostTaskLockKernel` wrapper using `Mutex::try_lock`. If the serialized arbiter is busy, return `AdmissionBusy` rather than parking a scheduler carrier. This is internal host metadata locking, **not** a guest `Mutex<T>` implementation.
- Tenant STOP is a trusted operation that cancels all registered waiters and TaskScopes first, and then closes queues. It never unlocks an active lease until trusted safe-point cleanup; it does not certify retirement.
- TaskScope and mutex grant generations are unique per kernel and per lock. A valid grant for mutex A is rejected when presented for mutex B; duplicate generations are rejected during lock registration.
- `WaitQueue::release` now requires a **claimed active guard**. A granted-but-unclaimed ticket cannot be passed through active-guard release; it must be revoked through cancellation. Existing host queue tests were adjusted to claim their grants before releasing them.
- `src/lib.rs` includes the private kernel only for **native host compilation** via `cfg(not(target_arch = "wasm32"))`, excluding the host synchronization structures from the browser compiler target. The kernel remains **unreachable from guest code**, and there is no scheduler callback providing authenticated guest-exit or notification-drain receipts.
- The model currently contains **35 authored Rust unit tests**. Rust and compiled guest verification remain `NOT_EXECUTED` because checked GitHub Actions jobs have zero recorded steps. The compiler's existing guest fast-path does not admit general task cancellation, contention, RwLock or actor critical-state guard paths.

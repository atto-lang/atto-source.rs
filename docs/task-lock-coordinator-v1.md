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


## Scheduler wake notification hardening (2026-11-20)

Host-only kernel `HostTaskLockKernel` now exposes separate bounded wake-hint operations:
- `cancelled_continuation_wake_hints()` is **level-triggered** until an authenticated task-exit event, so a waiting task cancelled while its mutex was held can be rescheduled without unlocking the holder.
- `pending_grant_wake_hints()` is **level-triggered**, not destructive; a failed scheduler ready-queue enqueue must not lose the lock grant notification.
- `acknowledge_grant_wake_hint(lock_id,ticket)` clears only the exact current grant notification after trusted scheduler ready-queue admission. Stale acknowledgments cannot clear a successor's notification. Actual lease entry always revalidates generation, ownership and cancellation via `claim()`.
- Wake-query calls use `try_lock`, returning `AdmissionBusy` instead of blocking an OS/scheduler carrier. They never publish Task DONE or release a guarded lease.

The unit-test source contains 40 `#[test]` cases at the latest update, including three new wake-notification regressions (queued cancellation, cancel-after-grant stale wake, and nonparking query contention). **These tests are authored, not yet executed on the current source head.**

CI carrier `benefactor-cc/atto-ci#6` has an exact mirrored Rust blob and pinned workflow. Its initial [Actions run #44682496723](https://github.com/benefactor-cc/atto-ci/actions/runs/44682496723) failed with **no job steps**; no compiler or host test result exists. Keep guest admission and all ten backend release flags disabled until proven.


## Follow-up audit: lexical TaskScope return and guard-scope exit (2026-11-20)

**Two verified-by-source, unexecuted runtime model changes:**

1. `TaskScopeLockCoordinator::guest_exited_at_safe_point` now marks the parent's terminal success/cancel winner, fences new child admission, and automatically requests cancellation of all unfinished children and grandchildren on ordinary **successful parent return** as well as on explicit cancel. Parent guard cleanup proceeds, then physical retirement remains gated on descendants' callback-drained quiescence. Added regression `scope_exit_auto_cancels_unfinished_descendants_without_clobbering_success`; siblings outside that scope remain healthy.
2. `TaskLeaseStack::release_top_guard`, `TaskScopeLockCoordinator::guard_scope_exit`, and `HostTaskLockKernel::trusted_guard_scope_exit` now model a **trusted lexical using-guard boundary** distinct from task exit. A claimed guard is released after its protected region ends, allowing a waiting successor to enter while the previous task keeps running. Only the top nested guard may be released (otherwise `GuardOrder`); successful release removes the ledger entry so cancellation cannot double-unlock. Stale/double release fails closed and fences retirement. These are trusted host-side model events, **not guest-callable methods or a proof that code actually left its critical region**. Three new regression tests cover early handoff, nested LIFO/duplicate guard release and post-terminal denial.

**Current source:** refer to exact draft [atto-source.rs PR #416](https://github.com/atto-lang/atto-source.rs/pull/416) HEAD and its `src/mutex_wait_queue_v1.rs` blob at test time. During this audit other commits landed on the same draft branch; always re-check live SHAs before testing or mirroring.

**Alternate GH Actions attempt (source-only model):**
- The temporary private [honeypot-r-us-test carrier PR #3](https://github.com/honeypot-r-us-test/atto-ci/pull/3) mirrored the then-current source file as exact Git blob `681238fa9b9dfa13cc3774dd61dbfed485f3fa03`, with zero-dependency Cargo harness and an SHA-gated workflow. [Run #44682861661](https://github.com/honeypot-r-us-test/atto-ci/actions/runs/44682861661) failed **before recording any job steps**. Not a compiler/test failure, and the snapshot predates the latest lexical-guard patch. Carrier PR closed unmerged and its branch reset to the base SHA.
- Temporary [voxletra capacity probe PR #12](https://github.com/voxletra/atto-ci/pull/12), [run #44682989587](https://github.com/voxletra/atto-ci/actions/runs/44682989587), also failed before job steps. No source copied; PR closed unmerged and branch reset.
- Other prior temporary carriers likewise failed before job steps. **No claim of available GHA budget, Rust compilation, sanitizer pass, guest conformance or backend parity.**

**Source-conformance checks performed:** current atto-platform-conformance #115 main registry has 32 SHA-pinned `.atto` fixtures and 13 target lock-lifecycle runtime probes. **All 32 source fixture SHA pins matched GitHub readback** (16+16 checks), every case has a valid expected category, IDs are unique, and ten backend cells correctly remain `NOT_EXECUTED`. This is source integrity, NOT actual `node --test`, compiled Atto, compiled Rust or runtime lock safety.

**Next release gates:** obtain an Actions runner that actually starts steps, regenerate a fresh exact-sha mirror of the **latest** source, execute `cargo test --offline --locked --lib`, optimized tests, Clippy and sanitizer/model checking; then integrate the trusted guard-scope end and TaskScope descendant cancellation into an authenticated, versioned production guest ABI. Run real compiled-guest scheduler traces before changing backend statuses or merging.


## Scheduler-authenticated lock parking and resume (2026-11-20)

The private host-side coordinator no longer infers that a task is suspended merely from its existence in a mutex wait queue. `park_lock_wait(task, lock_id, ticket)` is a trusted, serialized scheduler transition and requires:
- exact task/tenant/actor/incarnation identity and a still-queued matching ticket;
- no active scoped guard and no cancellation/physical exit;
- a task not already parked on another wait.

Cancellation wins before the park transition: the still-running task must observe cancellation itself; **no cancellation wake** may be issued. A successful park wins before cancellation: `cancelled_continuation_wake_hints()` returns a durable wake until trusted task exit. A grant that wins before park is claimed directly by the still-running task, never scheduled again. Grant notifications are visible only for actually parked tasks. A parked task cannot claim its grant until the trusted host ready-queue admission is acknowledged with its exact ticket, and a parked but unresumed task cannot certify successful physical exit.

Added adversarial Rust tests to the host model for pre-park cancellation, forged/duplicate park attempts, attempting to park under a live guard, grant-before-park, grant-ack ordering and premature retirement. **Current source has 48 authored host tests but none have executed at the exact head.** This remains a private native-host model; actual production guest scheduler callbacks and compiled `.atto` cancellation are still absent. Conformance registry in `atto-platform-conformance#115` now includes a 14th park/cancel race probe, with ten runtime backend cells kept NOT_EXECUTED.

The previous CI carriers did not admit jobs. [benefactor-cc/atto-ci#6](https://github.com/benefactor-cc/atto-ci/pull/6) mirrors the model source with a pinned Git blob; [run #44684958402](https://github.com/benefactor-cc/atto-ci/actions/runs/44684958402) failed with zero job steps. Do not mark Cargo tests or guest execution as passing.

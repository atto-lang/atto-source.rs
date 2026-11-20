# AttoLang actor-task parity — Rust compiler substrate (2026-11-12)

**Status: host-side lifecycle model only. NOT compiler-backed actor task parity.**

The mainline Rust/Cranelift parser currently compiles the scalar subset and deliberately rejects actor/task/suspends/await source. The `actor_task_lifecycle` module introduces a bounded per-actor task ledger for a future trusted scheduler integration, without inventing a compiler, a carrier pool, or a guest executor.

Contract implemented in the isolated Rust substrate:
- Parent task owns children even after its result settles; cancellation propagates to all still-live descendants.
- `stop` seals admission and requests cancellation but cannot forge physical task termination.
- A running callback is not cleanup-retirable until it actually returns. A suspended/queued task requires explicit trusted cleanup.
- Every task belongs to an actor incarnation; stale generations fail closed; bounded slots are released only by explicit retirement.
- Completion means the ledger is empty, not merely that all Futures have resolved.

**Not implemented:** compiled `.atto` actor task syntax, task scopes, authenticated physical actor turns, guest continuation frame lowering, carrier scheduling, quotas/fuel, and independent native execution tests. API entry points marked `admit_in_authenticated_turn` are **trusted-internal only**; the ledger does not authenticate a caller or execute guest code. No untrusted source may invoke them.

Execute once a funded runner is available:

```sh
cargo test --locked --lib actor_task_lifecycle -- --nocapture
cargo test --locked --test execution_domain_admission
cargo test --locked
```

Require exact-head SHA/tree and non-zero executed steps, counts, real compiled AttoLang guest tests after implementing lowering, and independent approval. `steps=null` means NOT_EXECUTED. Private source cannot be mirrored to arbitrary organizations without policy authorization.

## Authenticated WAKE integration gate (2026-11-12 update)

A suspended task no longer restarts through ordinary `start_or_resume`; that path returns `WakeRequired`. `suspend` records a monotonically increasing per-task suspension generation and returns a private-field `TaskWakeTicket` bound to the exact actor incarnation and task ID. `resume_from_authenticated_wake` checks identity, generation, stopped/cancelled state, and single-writer ownership; the ticket is consumed by the state transition. Stale, replayed, foreign, cancelled or STOPped wakeups fail closed.

**Security boundary:** This ticket is a logical replay fence, **not an unforgeable carrier capability**. Trusted scheduler code must authenticate the physical actor WAKE lease and external Future readiness before calling the ledger. Until it does, this remains a host model, not guest task resumption.

Authored Rust unit regression cases include wrong-task, stale-generation, repeated-suspension replay, STOP while awaiting, nested cancellation, quotas and single-writer serialization. No local Rust toolchain or executed Rust CI is available, so these are NOT_EXECUTED.

## Audit and hardening — 2026-11-12

Nested task admission now requires the exact parent task to be `Running`, not merely alive. This prevents a suspended/settled parent from being used to mint child tasks through a late Future callback. Added `parent_must_be_actively_running_to_admit_nested_task` regression, including ready, suspended, completed parent and same-actor exclusivity cases.

The logical wake ticket now requires a valid matching task, actor incarnation and per-suspension generation; ordinary `start_or_resume` rejects suspended tasks. This is **not** a cryptographic capability and does not prove physical actor WAKE ownership without a trusted scheduler.

Current implementation blob: `9394b2ff1fba07dd396aac3482a602dd151a2e5c`. Embedded lifecycle unit tests: **8 authored, NOT_EXECUTED**; local Rust toolchain unavailable. Private CI export allowlist has no approved destinations, so no out-of-org source transfer was attempted. Exact-head Cargo execution and compiler-backed `.atto` guest tests remain required.

## Task-identity side-channel audit

The original Rust model checked whether another task was running before verifying that the incoming TaskId existed. A guessed ID could therefore receive `ActorBusy` instead of `MissingTask`, exposing per-actor scheduling state. Both ordinary task start and wake-resume now validate task presence, lifecycle and wake identity before the single-writer busy check.

Unit tests currently authored: **9**, all **NOT_EXECUTED** without a Rust toolchain/approved CI. Implementation blob `19528a03cf3ff69a349c9cb21e8c3ebe00240bd8`.

## Two-phase physical suspension handshake (2026-11-13)

The bounded Rust host ledger now distinguishes `TaskStage::Suspending` (a guest callback requested suspension but its physical carrier lease has not been confirmed returned) from `TaskStage::Suspended` (the trusted scheduler acknowledged callback exit). A still-`Suspending` task blocks sibling mutable actor turns, cannot receive WAKE, and cannot be cleanup-retired merely because STOP was requested. `confirm_suspended_turn_returned(owner,id,ticket)` transitions the exact incarnation/task/suspension generation once. It is **trusted scheduler-only bookkeeping**, not authenticated by the Rust model itself and not an LLVM/Cranelift guest integration.

Ten embedded Rust unit tests are authored, including a new STOP-before-physical-exit and replay/duplicate-receipt case. **Not executed**: this environment has no Rust compiler; the requested source-local canary rerun `43333733456` was accepted but new job `131585208426` recorded **0 executed steps**. The latest ledger blob is `25fd9d30a44cf6918d45cf928da379f9ad3cd345`. No private-source cross-org mirror is approved.

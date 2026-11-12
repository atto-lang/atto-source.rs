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

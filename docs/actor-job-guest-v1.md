# Job<int> source-to-Wasm candidate — opt-in guest ingress v1

**Status:** executable, browser/tenant-local, volatile preview; NOT a
durable job broker, a production security ABI, or cross-backend parity.
This implementation is stacked on [atto-source.rs#398](https://github.com/atto-lang/atto-source.rs/pull/398)
and [#310](https://github.com/atto-lang/atto-source.rs/pull/310).

## Compiled source syntax

```atto
define serial actor Worker as
  receive(Job<int> job): void {
    stdio.println(job.job_id);
    stdio.println(job.correlation_id);
    stdio.println(job.attempt);
    stdio.println(job.payload);
    job.ack();
    return;
  }
end
```

`Job<int>` is a sealed receive-only binder; no general source-visible
`Job<T>` expression, constructor, class, ordinary function parameter or
network transport is implemented. Immutable `job_id` and
`correlation_id` are strings; `attempt` is nonnegative i64 and
`payload` is signed i64. `ack()` is a void request to the active actor
host, not a method on ordinary `ActorMail<T>`. `Job<bool>`, arbitrary ADT
payloads and foreign type payloads are rejected with no guest artifact.

## Separate, opt-in guest contract

Legacy v2 `actor_receive(i32 type, i64 value, i64 sequence)` remains
unchanged and **MUST trap for a Job<int> handler**. Modules using Job<int>
add a separate `actor_receive_job(i32 type, i64 value, i64 sequence,
i64 job_id_descriptor, i64 attempt, i64 correlation_descriptor)` export,
an `atto.actor_job_ack(i64 sequence, i64 attempt)` import when `ack()`
is used, a custom section `atto.job.abi` with exact bytes `01 00`,
and `atto.job.actors` with exactly one zero/one byte for each declared
actor in `atto.actors` order.

This is **orthogonal** to the separate proposed `atto.actor.abi` v3
continuation/scheduler design. Version 0 experimental `ATJB` frames in
`job_envelope.rs` are *not* the Wasm calling convention and are not
implicitly decoded by guests.

## Bounded identity memory and lifetime

Each job-bearing guest has a private minimum two-page (128 KiB)
non-growing Wasm memory. The first 64 KiB is the compiler static
data/heap region; when an allocating heap exists, its bump allocator
is capped at 64 KiB for a job-bearing module (others retain their
normal heap limit). The next 64 KiB, offsets `[65536,131072)`, stores
append-only UTF-8 metadata. Host descriptors are `offset << 32 | len`,
with ID length 1..4096, correlation length 1..256, and both pointers
and their lengths bounded inside that 64 KiB region.

**Entries are never reused while their actor is live**. A retained
`string` descriptor therefore cannot silently refer to a later job ID.
Metadata exhaustion is an explicit fail-closed runtime error and
previously admitted work is indeterminate, not silently replayed.
The actor's heap and metadata are freed on actor shutdown, not per job.
No raw pointers or cross-tenant references are exported.

## ACK and failure model

`job.ack()` calls the host **only during a matching current receive**.
The tenant host checks that the active actor is live, the exact FIFO
sequence and attempt match, that the job came from an opaque registry
claim, and that no previous ACK was accepted. An ACK is provisional
until the Wasm handler returns normally and host memory/output checks
pass. Traps, actor retirement, malformed claims or missing metadata
never turn into completed receipts. Normal return without ACK is
only host-observed execution.

A guest can ACK its own work before an external side effect commits;
that is an application-level declaration, not durable proof. This
v1 contract makes **no exactly-once claim**. Crash-durable deduplication,
external side-effect transactions, authenticated cross-process proxy
ACKs, cancellation, multi-backend parity and structural ownership
analysis remain future work.

## Executed verification

`tests/actor_job_guest_v1.rs` checks compiler construction of actual
deterministic Wasm plus missing/invalid feature rejection. The matching
[playground host integration](https://github.com/atto-lang/atto-playground/pulls)
builds this compiler from its exact source and executes real Job<int>
Wasm against the tenant scheduler, including 1,000 jobs, an ACK
followed by a trap, mixed legacy handlers, foreign/forged ACKs, and
retained ID lifetime. A full CI receipt is required before readiness.

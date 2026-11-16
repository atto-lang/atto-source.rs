# Worker-owned `Channel<int>` guest ABI — compiled Rust/Wasm v1 draft

**Status: Rust/Wasm compiler-generated channel imports and typed guest await
executed successfully in a real Chromium Worker on 2026-11-16.**
Source commit `9863f417cbd012d08312385e2262d2737bba5624`, exact source Git tree
`b5aa99efa84b53eac3c6ff0626a27a0cb5d5614f`, verified private runner
`anticaptrad/atto-ci` Actions #43949956999. This is a draft, opt-in
`atto.channel_int` v1 plus `atto.async` v2 profile, not stable public ABI
or cross-backend parity.

## Source behavior to implement

```atto
pub susp fnc main(): int {
    val channel = Channel<int>.bounded(1)?;
    val pending: Future<Result<Option<int>, IoError>> = nb readch channel;
    channel.send(41)?;
    val read: Result<Option<int>, IoError> = await pending;
    ^ match read over
        on Ok(Some(value)) -> value + 1;
        on Ok(None) -> 0; // EOF is NOT a value
        on Err(error) -> { raise error; }
    end;
}
```

This example remains **proposed surface syntax**: the browser currently
supports explicit `Channel<int>.bounded(literal)`, `nb readch`,
`await` and nested `match` on typed Result/Option, but does not yet
admit this example's `?`, nested `Ok(Some(...))` arm or `raise` syntax. The existing `Future<int>` task handle is NOT a substitute
for `Future<Result<Option<int>, IoError>>`: it cannot faithfully distinguish
EOF, cancellation, closed channels, quotas, and read errors.

`nb readch` must return immediately with an owned pending Future. Awaiting
may suspend the current structured task but must not park its Worker; the
existing mandatory `cooperate` turn applies even to immediately resolved
futures. No `await` is allowed inside a critical actor-state lease. No
second consumer may claim a Future, and no typed read may be silently
reinterpreted as a scalar `int`.

## Host reference model now provided

`runtime/browser/channel_int_v1.mjs` is the Worker-owned *implementation*
of bounded FIFO admission, pending read registration, checked ticket
consumption, EOF, closed-channel errors, cancellation, and quotas.
`tests/channel_int_host_v1.test.mjs` proves the corresponding host contract
with real asynchronous Promise continuations and i64 BigInt payloads.
Compiler-emitted guests now import this runtime only after opt-in marker
and exact Wasm function-signature validation; unversioned guests remain
unable to acquire channel imports.

A closed channel is not automatically destroyed, because queued payloads
remain readable until EOF. The host exposes explicit `release(handle)` only
when the channel is closed, its queue is drained and no registered read Future
remains outstanding. This prevents permanent channel-quota consumption while
protecting unconsumed values and pending Future owners. Premature release
returns checked `NotClosed` or `Busy`; released handles are never reused
and become invalid.

The host model accepts only signed 64-bit `bigint` elements, supports
zero-capacity rendezvous, and bounds channels (default 64), tickets (1024)
and per-channel capacity (up to 1024). It never speculatively reads ahead,
drops queued values on close, transforms an EOF into 0, duplicates admission,
or delivers a cancelled registration. Cancellation after successful send
cannot retroactively revoke a committed value. Read ticket IDs are local
to one Worker runtime and never reused; no cross-Worker or actor isolation
permission is conveyed by a numeric ID.

## Implemented versioned Wasm import bridge — draft opt-in

A compiled guest that uses channel operations carries a custom capability marker
`atto.channel_int` with version 1 and adds only explicitly used imports.
The existing async marker `atto.async` version 2 must remain compatible.
The bridge requires a concrete, stable type representation before admission:

| Import candidate | Params | Result | Purpose |
|---|---|---|---|
| `atto.channel_int_open_v1` | `capacity: i32, out: i32` | `status: i32` | Checked channel creation |
| `atto.channel_int_send_v1` | `channel: i32, value: i64` | `status: i32` | Exactly one FIFO admission |
| `atto.channel_int_nb_read_v1` | `channel: i32, out: i32` | `status: i32` | Nonparking read registration |
| `atto.channel_int_await_read_v1` | `ticket: i32, out: i32` | `status: i32` | JSPI one-shot await/settlement |
| `atto.channel_int_cancel_v1` | `ticket: i32` | `status: i32` | Checked cancellation |
| `atto.channel_int_close_v1` | `channel: i32` | `status: i32` | EOF while preserving queued values |
| `atto.channel_int_release_v1` | `channel: i32` | `status: i32` | Release a closed, fully drained capability |

Names and layouts are **implemented in this Rust/Wasm draft** but have not
yet been declared a stable public ABI or proven on Java, LLVM, or .NET.
Every output pointer must be bounds/alignment checked against the same
Worker-owned guest linear memory, with integer overflow guards. The
read-result slot must encode exactly one tag from
`Ok(Some(i64))`, `Ok(None)`, or `Err(IoError)`; no sentinel i64 can
replace the tag. Errors are typed, versioned and nonthrowing for ordinary
closed, cancelled, quota, or invalid-capability results.

**Host-only abandoned-Future disposal (not guest ABI v1):**
`ChannelIntHostV1.abandon_read(ticket)` detaches an uncommitted pending
registration, retires its ticket, reclaims the ticket quota, and wakes any
already-claimed pending await with `Err(Cancelled)`. It also retires an
unclaimed settled EOF/error ticket. A ticket holding an already committed
`Some(value)` returns `Busy`: losing an admitted value to an ignored Future
is forbidden. `cancel(ticket)` is distinct: it records a cancellation result
but intentionally leaves an unclaimed ticket available for a later checked
`await_read(ticket)`. The compiler does **not** yet emit `abandon_read` on
`^`/`return`, exception unwind or scope exit, so automatic guest cleanup and
linear ticket ownership remain **unverified and merge-blocking**. A future
versioned ABI must provide the appropriate drop/abandon hook and prove every
control-flow exit executes it without losing committed values.

Release/merge gates (the source typing, tagged Wasm lowering, marker checks,
and single-fixture Chromium execution have passed; the broader lifecycle and
cross-backend gates remain):
1. The checker can represent `Channel<int>`,
   `Future<Result<Option<int>,IoError>>` and the complete awaited
   `Result<Option<int>,IoError>` ADT type.
2. The checked Wasm IR constructs that ADT without leaking a host pointer,
   fabricating an Error subclass, or allowing two reads of one Future.
3. The host validates its source Worker, channel owner, ticket lifecycle,
   result-slot bounds, int range, and capability marker before interaction.
4. `task_await` and channel-read awaits cannot collide in one handle
   namespace, and JSPI suspension/resumption is race-free.
5. Full compiler-backed guest tests cover immediate/pending reads, FIFO,
   two readers, capacity zero/full, EOF, closed/error/cancel, quotas,
   close-vs-commit and cancel-vs-commit races. They must observe one
   cooperative scheduler turn, no double delivery, and no spurious
   supervisor CONTROL messages.
6. Native/wasm32 byte-equivalence and Chromium Worker execution pass on
   the **same exact Git tree**, with SHA-verified source and a real runner.
7. Java, LLVM, .NET and other backends separately prove the normative
   error/ownership/await contract; Rust/Wasm PASS is not automatic parity.

## Directional and continuous grammar remains disjoint

`source >> val item` and `sink << value` describe exactly one operation.
`nb source >> val pending: Future<Result<Option<int>,IoError>>` registers
one checked read only. `source pipe sink` is continuous bounded transfer;
it needs independent graph lifecycle, backpressure, close, error routing
and typed actor sends/receives. The browser compiler currently recognizes
`pipe` but must return `E_UNSUPPORTED` rather than emit a fake stream
runtime.

Host reference model source and tests must be treated as **contract tests**
until the bridge gates above are implemented and independently verified.


## Executed implementation staging — November 16, 2026

The adapter at runtime/browser/channel_int_guest_v1.mjs now defines a
checked import table, error/status codes, and a strictly tagged 16-byte
read outcome slot: at offset 0 an i32 tag (0 Some, 1 EOF/None, 2 Error),
at offset 4 an i32 IoError code (zero on success), and at offset 8 an
i64 payload (meaningful only for Some). Output pointers are checked for
linear-memory bounds and 8-byte alignment on awaited read slots; handle
slots require 4-byte alignment. No raw host pointers cross the ABI.

The adapter also defines channel_int_release_v1(channel: i32) -> i32
to reclaim explicitly closed, drained, ticket-free channel capabilities.
Numeric handles are scoped to a single guest Worker adapter; the import
signature validator checks exact Wasm function types and fails closed
on missing/duplicate/incorrect capability markers or forged imports.

Native Node tests now cover this typed memory layout, the three
Some/None/Error outcomes, cancellation, read ticket replay, pointer
safety, lifecycle and a genuine Wasm import module. The dedicated
Chromium Worker probe uses a synthetic Wasm import module, not an
AttoLang-compiled Channel<int> guest. This distinction is intentional.

Since source commit 9863f417, compiler source typing, JSPI await lowering,
tagged ADT construction, source-level `nb readch`, and versioned import
markers are executable and CI-verified in Chromium. The next gates include
compiler-generated error/EOF/cancellation/close/release adversarial execution,
linear ownership and abandoned-Future cleanup, and separately verified
Java/LLVM/.NET semantics. The host synthetic-Wasm probe is additional
evidence, not a substitute for the compiled guest test.

## Cancellation from compiled guest (draft)

`Future<Result<Option<int>,IoError>>.cancel()` is a checked Worker-local
operation returning `Result<void,IoError>`. A successful cancellation
settles the pending read with `Err(Cancelled)`, but **does not consume**
the Future. The owner must still `await pending` once to retire the ticket;
channel `release()` remains `Err(Busy)` until that happens. Cancellation
after send commit returns `Err(AlreadySettled)` and the pending Future
still delivers its original `Ok(Some(value))`. Cancellation does not
silently erase committed FIFO messages.

An abandoned Future without awaiting or explicit drop remains a lifecycle
gap; the compiler does not yet enforce linear cleanup on all return paths.
Keep the language/runtime contract in draft until that ownership obligation
is checked and tested across all exit/throw/cancellation paths.

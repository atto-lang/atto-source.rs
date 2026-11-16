# Worker-owned `Channel<int>` guest ABI — staged v1 contract

**Status: host runtime and reference tests implemented; compiler-to-guest
channel ABI NOT yet admitted.** A passing JavaScript host model is not a
compiled AttoLang guest execution receipt. Avoid falsely promoting this to
`atto.browser.guest.v2`.

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

This example is **proposed**, not supported source syntax on the current
browser compiler. The existing `Future<int>` task handle is NOT a substitute
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
The runtime is **not imported by guest Wasm yet**; there are no silent
channel imports exposed to unversioned guests.

The host model accepts only signed 64-bit `bigint` elements, supports
zero-capacity rendezvous, and bounds channels (default 64), tickets (1024)
and per-channel capacity (up to 1024). It never speculatively reads ahead,
drops queued values on close, transforms an EOF into 0, duplicates admission,
or delivers a cancelled registration. Cancellation after successful send
cannot retroactively revoke a committed value. Read ticket IDs are local
to one Worker runtime and never reused; no cross-Worker or actor isolation
permission is conveyed by a numeric ID.

## Proposed versioned Wasm import bridge — NOT yet exposed

A future accepted guest carries a custom capability marker
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

Names and layouts are **candidate designs**, not a shipped public ABI.
Every output pointer must be bounds/alignment checked against the same
Worker-owned guest linear memory, with integer overflow guards. The
read-result slot must encode exactly one tag from
`Ok(Some(i64))`, `Ok(None)`, or `Err(IoError)`; no sentinel i64 can
replace the tag. Errors are typed, versioned and nonthrowing for ordinary
closed, cancelled, quota, or invalid-capability results.

Do not add imports or the marker to guest modules before:
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

# Rust/Wasm v2 native Atto `std/channel` source extraction

The v2 in-guest runtime is now composed from
`src/v2/prelude.atto` plus `stdlib/channel.atto`. The latter is
a separate compiler-backend-owned **Atto source file**, not a JS queue,
host function or second compiled language. `src/v2/mod.rs` inserts its
contents at the unique `@atto-stdlib-channel-source` marker before
parsing and compiling the guest runtime as a unit.

## Evidence that source semantics were preserved

The original extraction of 12 `__chan_*` routines reproduced the previous
inlined source exactly. This proof belongs to the initial refactor commit;
subsequent hardening intentionally changed the composed guest runtime bytes. The assembled prelude's Git blob SHA-1 equals the
pre-extraction `src/v2/prelude.atto` blob:

```text
cbbc96d544563d27953796b3a49e88ea43ec0523
```

This was an exact **pre-hardening source-byte equivalence**, not an executed
compiler or guest Wasm receipt. The post-hardening source is intentionally
different, so the old digest is **not** a current-head equivalence receipt. The branch must still pass its full compiler,
fixture, guest, leak/cancellation and actor-pipe tests at its new HEAD.
The optional static source guard (now also checking the bounded admission
route and compiling three capacity-related Wasm fixtures) is
`cargo test --test v2_channel_stdlib_source`; the new guest fixture is
`tests/fixtures/pass/235-v2-channel-module-fifo.atto`.

## Bounded capacity safety hardening

The legacy guest constructor treated every negative capacity as effectively
unbounded. As a result, `Channel<int>.bounded(-1)` silently acquired the
privileged unbounded semantics. The compiler now lowers both `Channel<T>.bounded(n)` and the legacy
`channel<T>(capacity: n)` constructor to
`__chan_new_bounded(n)`, implemented in this backend's `.atto` module.
This helper rejects all negative values before allocating. Direct internal
`__chan_new` rejects values below -1, while the dedicated
`unbounded()` constructor still passes -1. Guest denial uses the distinct
panic code **14**. This is a runtime trap, not yet the final recoverable
`Result<Channel<T>,ChannelError>` contract.

New guests:
- `tests/fixtures/panic/235-v2-negative-bounded-channel.atto` (panic 14)
- `tests/fixtures/panic/236-v2-negative-other-capacity.atto` (panic 14)
- `tests/fixtures/panic/237-v2-negative-legacy-channel-constructor.atto` (panic 14)
- `tests/fixtures/pass/236-v2-unbounded-constructor-atto.atto` (exit 42)

Compiler-to-Wasm assertions are not browser guest execution. All new current
head native/Rust/Wasm conformance remains NOT_EXECUTED until verified.

## Cooperative zero-capacity rendezvous

Audit uncovered a deadlock in the original v2 runtime: `__chan_send` can
handoff on capacity zero only to a registered `nb readch` waiter, while
ordinary `readch` and `for await` originally suspended in a polling loop
without registering a receiver. A producer and reader could both suspend
forever. The backend-owned `stdlib/channel.atto` now registers one
`__chan_nb(c)` future for an empty capacity-zero channel, awaits its
completion through guest `__await`, and checks both DATA graph ownership
ports again before using the result. Ordinary `readch` reports closure
as panic 9; `for await` receives None to terminate iteration cleanly.

The three regression guests are:
- `tests/fixtures/pass/238-v2-rendezvous-direct-readch.atto`
- `tests/fixtures/pass/239-v2-rendezvous-for-await.atto`
- `tests/fixtures/pass/240-v2-rendezvous-close-wakes-reader.atto`

`tests/node-v2-channel-guest.mjs` compiles and executes **15** positive and
negative channel fixtures (six successful, nine expected guest denials)
as actual independent Wasm guests. These now cover direct rendezvous,
for-await rendezvous, close/wakeup, invalid capacities, legacy two-port
actor pipe ownership admission, and closed-versus-empty immediate read.
Each guest runs
in a bounded child process; a hang, nonzero child exit, incorrect stdout,
wrong return value, or wrong panic code **fails** the gate. The harness is
wired into the Rust `.github/workflows/ci.yml` after the native `attoc`
build. A source-free/zero-step Actions result is **NOT_EXECUTED**, not a
pass; these new tests still await stepful admission on the exact PR HEAD.

Pending cancellation and removal of abandoned `nb readch` futures remain
unverified. This milestone fixes cooperative rendezvous progress, not
fully lifecycle-safe cancellation or Java/Rust public API parity.

## ABI and unsupported APIs

`__chan_new(-1)` is an in-guest growable queue sentinel. Capacity zero
is rendezvous, positive capacity is bounded. `__chan_send` and
`__chan_recv` suspend the cooperative guest task. Closed channels
drain buffered values, and after drain reads terminate by the v2
panic-code contract. Direct source reads/writes fail closed when the
channel has been claimed by an actor DATA graph.

**This is the backend-private runtime portion of std/channel.**
It does not yet expose the Java `import module channel from
"std/channel"` API through the Rust v2 package linker; v2 only admits
the specifically enumerated package imports. Generic ownership,
TryWriteResult<T> Full/Closed recovery, faithful cancellation of
pending registered operations, cross-tenant transports and true
unbounded resource quotas are NOT VERIFIED here.

The Java/Graal implementation is independently source-owned in
`atto-source.java` PR #1783. No byte-identical library implementation
is required between compilers; identical public behavior is a
future conformance target.

## Legacy actor-pipe admission and closed immediate reads

The checked three-part `PipeHandle` path previously refused a buffered
channel or pending public `nb readch` Future. The older two-part actor-pipe
helpers (`__actor_pipe_channel` and `__channel_pipe_actor`) lacked these
checks. Both now fail closed with guest panic code **10** if the channel
has buffered data or registered public readers, *before* either actor
DATA port is claimed. Four guest denial fixtures cover both directions
and both admission hazards.

`try readch` on an open, temporarily empty channel continues to return
`None`. Once a channel is closed and its committed FIFO has drained,
the same immediate read reports guest panic code **9**; it must not
pretend that terminal closure is transient emptiness. Three additional
guest cases cover open-empty, post-close drain, and terminal immediate
read, including stdout-before-denial order.

These improvements do **not** implement cancellation of pending `nb`
Futures. The v2 runtime currently does not expose a verified cancellation
token / removal protocol. Cancellation remains **NOT_IMPLEMENTED** until
registrations are safely unlinked under a defined scheduler contract.
There is no claim of guest PASS without stepful exact-head execution.

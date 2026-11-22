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
privileged unbounded semantics. The compiler now lowers `bounded(n)` to
`__chan_new_bounded(n)`, implemented in this backend's `.atto` module.
This helper rejects all negative values before allocating. Direct internal
`__chan_new` rejects values below -1, while the dedicated
`unbounded()` constructor still passes -1. Guest denial uses the distinct
panic code **14**. This is a runtime trap, not yet the final recoverable
`Result<Channel<T>,ChannelError>` contract.

New guests:
- `tests/fixtures/panic/235-v2-negative-bounded-channel.atto` (panic 14)
- `tests/fixtures/panic/236-v2-negative-other-capacity.atto` (panic 14)
- `tests/fixtures/pass/236-v2-unbounded-constructor-atto.atto` (exit 42)

Compiler-to-Wasm assertions are not browser guest execution. All new current
head native/Rust/Wasm conformance remains NOT_EXECUTED until verified.

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

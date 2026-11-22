# ActorFuture<int> source profile — first full-AOT actor response

## Language contract

`ActorFuture<T>` is an owned, single-reply future **bound to a producer
actor incarnation**. Unlike a generic `Future<T>`, it has producer
authority, actor shutdown semantics, and supervisor failure propagation.
**Streaming is not one-shot**: a separate `ActorStream<T>` will represent
multi-event replies with backpressure and cancellation; `ActorFuture`
must never implicitly drain it.

First executable source profile (not general actor messaging):

```atto
pub susp fnc main(): int {
  val first: ActorFuture<int> = actor_future_int(7, 41);
  await first;
  val second: ActorFuture<int> = actor_future_int(7, 42);
  return await second;
}
```

Compile with `attoc --actor-future-int-aot file.atto -o actor.wasm`,
`compile_actor_future_int_aot(source)` in Rust, or
`compile_actor_future_int(length)` in the wasm32 compiler.

The normal parser recognizes the distinct `ActorFuture<int>` type,
but its execution is admitted ONLY by the new explicitly selected
actor-future profile. This first profile requires exactly one
`susp main():int`, 1–8 straight-line awaits, and positive
compile-time actor ID and nonnegative i64 request-key literals.
There are at most 8 immutable scalar or future locals. Every
`ActorFuture<int>` must be immutable `val`, consumed by exactly one
`await`, never aliased, reassigned, leaked, dropped unconsumed or
returned without awaiting. Normal `Future<int>`, non-`int` generic
actor futures, branches/loops/calls, dynamic actor identity, untyped
scalars and `ActorStream<T>` fail closed.

The checked source frontend emits the already-executed portable-v5
guest AOT frame/state machine and a unique custom section:

- `atto.browser.guest.v5`: byte `05`
- `atto.browser.source.actor_future.v1`: byte `01`, count `u8`,
  then `count` entries of `actor_id:u32-le, request_key:i64-le`.

The entries are ordered by source await occurrence. The registered host
verifies this exact layout and each pending guest request key. Actual
actor **incarnation** is supplied and fenced by a trusted Worker host,
not by source code. The marker is a compile contract, not a signature.

The first runtime adapter lives in `atto-source.js` and is vendored
byte-identically at `runtime/browser/actor-future-host-v1.mjs`. It
registers an actor ID and strictly increasing incarnation, transfers
a unique reply MessagePort per task to that actor's dedicated Worker
control channel, and sends a single-use 128-bit completion bearer
for each pending request. Actor Worker responses cannot execute guest
code inline; only separate future-completion and resume scheduler
turns can advance the guest. On trusted host actor termination/panic,
the host first revokes outstanding futures and marks the old
incarnation invalid, then reports a supervisor notice. Old grants
or ports cannot resurrect a restarted actor.

## Remaining work

This does **not** yet emit a genuine `ActorRef<T>.send` operation or
full typed mailbox admission, implement `ActorFuture<Result<T,E>>`,
route a complete Atto Director supervision tree, provide general
`ActorFuture<T>` specialization, handle cross-actor awaits in one
task, or provide `ActorStream<T>` bounded demand and stream
termination. Actor reply validity relies on trusted host transport
and bearer possession; an attacker who steals a ticket can choose
its authorized result, so the actor Worker must be isolated and its
reply channel protected.

The full guest remains AOT-compiled with no JSPI, runtime guest
compiler, interpreter, or JIT. Require native Rust proof, real Node
compiled Wasm and real Chromium nested actor Worker execution on an
exact-tree alternative-org GitHub Actions runner. Both PRs stay Draft.

## Bounded explicit `take!` admission (2026-11-22, candidate)

The v1 actor-future compiler profile accepts `await take! reply` as an
explicit transfer of a **named, locally owned** `ActorFuture<int>` result
receipt. The checked frontend tracks the same single-use `await` state
as the existing bare `await reply` syntax; it erases the source-level
`take!` wrapper only after proving the owner and consumes exactly one
receipt, then lowers to the established AOT v5 pending-key frame.

Examples of the **narrow** compiled-source candidate:

```atto
pub susp fnc main(): int {
  val reply: ActorFuture<int> = actor_future_int(7, 41);
  return await take! reply;
}
```

Negative cases: re-awaiting after transfer, transfer of a scalar, an
unknown name, `take!` of a computed/temporary request, wrapping an
unproven borrow, and a `take !` with whitespace/comments/newline
before the `!`. No guest artifact may be emitted for any rejection.

This does NOT provide general generic `take!` formal passing,
`TaskFuture<T>`, `ActorFuture<Result<T,E>>`, source-imported native
stdlib helpers, typed actor message admission, or compiler support
for non-int `ActorFuture<T>`. Existing narrower actor ID, request key,
incarnation-fencing and trusted Worker-host semantics remain unchanged.

Evidence is pending exact-head native Rust tests, actual AOT guest
execution in Node/Chromium and separate independent backend receipts.
Do not count source-only inspection or zero-step CI as execution.

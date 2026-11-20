# Declared actor message contracts — Rust/Wasm frontend slice

**Status:** implementation draft, not verified guest conformance. Normative
semantics are tracked by [atto-spec #28](https://github.com/atto-lang/atto-spec/pull/28)
and the cross-backend cases by [atto-platform-conformance #33](https://github.com/atto-lang/atto-platform-conformance/pull/33).

## Accepted declaration shape (bounded subset)

```atto
define actor Router receives int sends int as
    susp receive(ActorMail<int> mail): void {
        stdio.println(mail.value);
        return;
    }
end

pub susp fnc main(): int {
    val router = spawn Router();
    router.send(42);
    return 0;
}
```

- `receives T` declares the input application payload type and must equal
  the existing `receive(ActorMail<T> mail): void` payload.
- `sends U` is an outbound application-message type **effect**, never
  a count or a destination declaration. An omitted clause remains the
  legacy implicit-effect mode.
- A `susp` marker is admitted on the illustrated handler and top-level
  callable only within this frontend's existing non-suspending executable
  subset. The parser must still fail closed on unsupported actual suspension.
- `ActorRef<Router>` retains its nominal actor identity; its `send`
  argument must match `Router`'s checked mailbox payload.
- A declared `sends U` checks typed `Send` expressions transitively through
  statically resolved helper calls, including branches not taken at runtime.
  Unknown call graph targets are rejected; there is no dynamic invocation
  widening in this subset.

## Explicit limitations

**This frontend's actor mailbox codegen currently supports only `int`
and `bool`.** In particular the requested
`receives string` + `ActorMail<string>` sample must produce
`E_UNSUPPORTED_TYPE`, not a runnable module. True string support requires
a validated length-delimited, ownership-safe message wire crossing worker
memories, host admission and guest decode with bounded quotas.

This restricted browser backend also rejects spawning actors *from within*
an actor, and rejects actor-reference types in actor method parameters.
Therefore the outbound effect analysis is a preventive type gate, **not**
proof of successful actor-to-actor outbound guest execution. It must be
independently exercised as the supported actor-ref and runtime subset expands.

`sends never`, union payloads, generic types, actor reply streams and
cross-isolate/remote serialization need their separately versioned support.
Supervisor priority CONTROL is runtime-authenticated and excluded from both
application clauses. No runtime topology, fan-out count or delivery ACK
guarantee is inferred from the declared types.

## Candidate UTF-8 DATA framing primitive (not connected to guest)

`src/actor_string_mail.rs` supplies a bounded, strictly validated host-independent
frame for future `ActorMail<string>` admission. The exact byte layout is:

| Offset | Field | Encoding |
| --- | --- | --- |
| 0–3 | Magic | ASCII `ATAM` |
| 4 | Version | `1` |
| 5 | Application payload kind | `1` = UTF-8 string DATA only |
| 6–9 | Payload byte length | unsigned 32-bit little-endian |
| 10 onward | Payload | exact, validated UTF-8 bytes |

A frame is limited to **65,546 bytes** including the ten-byte header; zero-length
strings are valid, while mismatched lengths, trailing bytes, wrong tags,
malformed UTF-8 and forged lengths must be rejected. The decoder returns an
owned `String` independent of the frame buffer. This deliberately grants **no**
mailbox, actor, tenant, sequence, or supervisor CONTROL authority. Distinct
Unicode byte spellings are not normalized or aliased.

This module is only a **candidate codec**, not a published network format or
source-level runtime ABI. It is not called by the Wasm compiler's actor send,
the Web Worker host mailbox, the worker memory copying bridge, or guest
`receive`. An acceptance of the original `receives string` source sample
remains **forbidden** until bounded cross-memory admission, guest decoding,
end-to-end ownership, and real compiled guest execution are implemented.
Do not reuse the separately proposed `Job<T>` envelope protocol as if it
were an `ActorMail<string>` DATA frame.

Standalone unit test (local Rust toolchain or authorized CI runner):

```sh
rustc --edition=2021 --test -D warnings src/actor_string_mail.rs -o /tmp/atto-actor-string-mail-tests
/tmp/atto-actor-string-mail-tests
```

**Evidence status: NOT_EXECUTED** until a trusted stepful runner produces
an exact-commit test receipt. Host codec tests do not prove guest transport.

## Verification gates

- A real pinned Rust compiler must build and compile the new positive fixture.
- The emitted Wasm guest must execute, printing 42, with exact module identity.
- The six negative fixtures must produce precisely the expected diagnostics
  with **no guest artifact**.
- CI/head SHA and toolchain must be recorded before claiming execution.
- Reconcile overlapping actor parser work in
  [atto-source.rs #297](https://github.com/atto-lang/atto-source.rs/pull/297)
  and the existing attached-receive
  [#216](https://github.com/atto-lang/atto-source.rs/pull/216)
  before merging.

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

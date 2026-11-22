# Typed `Future<int>` source constructor: experimental full-AOT v1

This milestone adds a **source-checked, single-use `Future<int>`** path
on top of the already verified portable-v5 *externally pending signed-i64*
guest continuation ABI. It does **not** implement arbitrary generics,
`Future<T>` for non-`int` payloads, dynamic future allocation, or
unrestricted async control flow. The full guest is AOT compiled.

## Opt-in source form

```atto
pub susp fnc main(): int {
  val key: int = 41;
  val first: Future<int> = pending_future_int(key);
  await first;
  val second: Future<int> = pending_future_int(42);
  return await second;
}
```

Native: `attoc --future-int-aot input.atto -o output.wasm`.
Rust library: `compile_future_int_aot(source)`.
Browser compiler WebAssembly: `compile_future_int(length)`.

The compiler parses `Future<int>` using its existing canonical generic
type parser, then validates the closed source profile *before* lowering
into the existing scalar `int` guest representation. In this stage
`pending_future_int(key)` creates an owned, lazily registered **external
key capability**. Actual suspension registration happens when awaited.
It is a compiler-recognized opt-in constructor, not a normal callable
and not a real runtime value that may be passed to other functions.

The accepted profile consists of exactly one `susp` main returning
`int`, at most eight simple immutable scalar or future locals, 1–8
straight-line awaits, and an explicit final `return await`. Pending
future keys are literal signed integer values or earlier initialized
scalar `int` locals; arbitrary arithmetic expressions and function
calls remain unavailable within constructors.

**Ownership invariants:** each `Future<int>` must be declared with
`val`, cannot be aliased, reassigned, passed into a scalar binding,
used as another future's key, dropped unconsumed, or awaited twice.
The source gate statically consumes it at its unique `await` site
and rejects subsequent uses with `E_MOVED`. It also rejects
unannotated constructors, plain `await int` under this target,
unsupported branches, loops, casts, captures, and all currently
unimplemented non-`int` `Future<T>` instantiations. Unsupported
source returns a diagnostic before guest bytes are emitted.

## Emitted guest and host semantics

The checked typed frontend lowers to the same underlying proven v5
eight-function Wasm lifecycle, private 16-slot arena, generation-fenced
handles, and external signed-i64 future completion protocol.

The guest retains the existing marker `atto.browser.guest.v5=05`
and additionally advertises `atto.browser.source.future.v1=01`.
The marker is a *source compilation contract*, not a cryptographic
signature. Default `compile`, `--portable-v4`, and
`--portable-v5` do not gain a new constructor and preserve their
previous artifacts and semantics.

The existing JavaScript `createPortableTaskHostV5` registers each
pending key with a single-use authenticated completion ticket and
dedicated Worker MessagePort. Each external completion only queues
a later guest completion and a separate resume scheduler turn.
The guest remains suspended without polling. Cancel and shutdown
revoke tokens, erase frames, and reject replay.

## Verification gates and remaining work

Required tests include `cargo test --locked --test portable_future_int`,
`node tests/portable-future-int-e2e.mjs` with native Rust `attoc`,
full Rust regression/conformance and wasm32 compiler built, and
real Chromium Worker tests using the wasm32-compiled Rust compiler.

The source gate is intentionally **not a replacement for general typed
IR ownership flow analysis**: future work must add a proper
`Future<T>` typed IR node, static constructor effects and ABI
specialization, full CFG/liveness spilling across branches/loops,
typed `Result<T,E>` pending failure, actor TaskScope/supervision,
and a contended, cancellation-safe async mutex. The current proof
is confined to its specified straight-line scalar profile.

Keep the compiler PR Draft until review and cross-backend validation
are performed. Do not enable this profile silently in the public
playground or claim parity beyond the verified Rust/Wasm guest + JS host.

## Bounded explicit `take!` await (candidate)

The typed `Future<int>` AOT v1 profile additionally recognizes
`await take! future_local` when `future_local` is a named, live
affine receipt created by the admitted `pending_future_int(key)` path.
The compiler consumes precisely the same right as ordinary
`await future_local` and erases the checked prefix before AOT lowering.
Two awaits (including mixed bare/explicit forms), arbitrary scalar
`take!`, borrowed/computed futures, and a separated `take !` marker
must be rejected without emitting an artifact.

This is **not** general `Future<T>` monomorphization, native .atto
stdlib generic function compilation, `TaskFuture<T>` control splitting,
or arbitrary result storage across `await`. Keep other profiles
fail-closed; native Rust/wasm32 compiler plus Node/Chromium compiled
guest evidence is required before calling this executable.


## Native generic Future await-adapter specialization — bounded implementation

Source-first native Atto supports the canonical generic signature as a
**closed-world, same-file, token-verified identity adapter**:

```atto
pub susp fnc await_future<T>(take! Future<T> receipt) -> T {
  return await receipt;
}
pub susp fnc main(): int {
  val receipt: Future<int> = pending_future_int(41);
  return await await_future<int>(take! receipt);
}
```

Only the exact `T=int` substitution, exact `pub susp fnc` identity-await
body, attached `take!` formal and explicit `await` call are admitted.
The compiler rewrites this one proven body into the already verified
single-use AOT await path before the ordinary parser, typed checker and
portable-v5 emitter. The checked Future provenance remains entirely under
the existing `future_typed_v1` single-observation gate; no userland runtime
code, closure, boxed generic value, JIT or dynamic dispatcher is introduced.

For actor replies, `await_actor<T>(take! ActorFuture<T> reply)->T` has the
same strict `T=int` instantiation on the dedicated ActorFuture AOT profile.
The actor incarnation and reply authority are still authenticated by the
trusted Worker host. The two profiles cannot be freely interchanged.

**Not yet supported:** imported/cross-file `.atto` modules, the complete
`define module future_family` library import, general callable instantiation,
`TaskFuture<T>`, `ActorFuture<Result<T,E>>`, streaming, arbitrary T, E,
control-right splitting, reference-bearing continuation frames, generic
combinators and independently linked object code. Any divergent adapter body,
unknown generic instantiation, implicit/borrowed argument, or double await
must fail closed with no AOT guest. Link to the separate borrower-checker
[PR #346](https://github.com/atto-lang/atto-source.rs/pull/346) when adding
general ownership-mode signature metadata.

Native Rust positive/negative and actual Node/Chromium AOT guest tests are
registered in the compiler repository; on-head execution is **pending**,
not proof. No external private-source carrier use is authorized without
[independent policy approval](https://github.com/atto-lang/atto-platform-conformance/issues/548).

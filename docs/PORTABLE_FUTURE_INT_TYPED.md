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

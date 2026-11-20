# Executable stateful method preview: Counter.increment

This PR introduces the **first actual source-level state-field operation** in
the experimental Rust/Wasm backend. It does not enable general production mtd.

Use `attoc --stateful-method-preview tests/fixtures/stateful/counter.atto -o counter.wasm`.
The wasm32 compiler exports `compile_stateful_method_preview(length)` for
byte-identical browser compilation.

## Admitted source subset

Exactly one defined actor with exactly one mutable `int` field, initialized by
a constant signed i64 literal; one mutable receiver with one `int` parameter and
return type `void`; one critical-write callback with **exactly** this state
assignment (whitespace and binder/field/param names may vary):

```atto
self.handle_state(critical-write |s| -> {
  s.count = s.count + delta;
});
```

This restricted frontend verifies the full target grammar, types, effect and
borrow facts, and source-located state-lease obligations. Its exact token-shape
gate rejects extra statements, different arithmetic, calls, suspension,
nested access, imports or incomplete actor declarations. It lowers the
**actual actor field initializer** and the precise `s.count += delta` effect;
unlike #160, no synthetic initial value of 41 is substituted. Checked i64
overflow leaves state unmodified and releases the guest-private lease before
trapping.

The guest exports:
- `atto.state.<Owner>.<method>.1(i64 delta, i32 cooperative_cancel)->i64`
  (the returned updated value is **preview diagnostics**, not a source-level
  `void` return)
- `atto.state.<Owner>.<field>() -> i64` (private diagnostic snapshot)
- `lease_active()`, `acquire_count()`, `release_count()`
- `close_witness()` and mutable `panic_code`

Emitted branch code shares #160's exact byte-level audit and strict cleanup CFG:
normal return, checked overflow, cooperative cancellation each contain exactly
one release. These guest flags are **not host-owned locks**. A separate
supervisor-owned lease registry is necessary to revoke authority on watchdog
timeouts and hard Worker termination.

The production `compile()` and `--receiver-instance-preview` paths continue
to reject stateful mtd, and `require_backend_cleanup_proof` remains closed.
This only establishes the first narrow vertically integrated execution path.
No full actor mailbox semantics, reentrancy, multiple state fields, nested
leases, general expressions, or unrestricted `critical-read` / `critical-write`
execution is claimed.

## Restricted attached-state source parity (2026-11-19)

In addition to the original inline actor-owned `let int count = 17;`
fixture, the `--stateful-method-preview` compiler now admits exactly one
**nominal attached** initialized scalar field:

```atto
use namespace counters;
declare actor Counter;
define actor Counter as
end
define state on actor Counter as {
    let int count = 17;
}
mtd (mut self: Counter) increment(int delta): void {
    self.handle_state(critical-write |s| -> {
        s.count = s.count + delta;
    });
}
```

The attachment is resolved to the sole declared and defined actor *after*
source declaration scanning. It is reparsed by the ordinary actor-state
frontend to retain `FieldMode::ActorState`, then subjected to the exact
typed state-use and audited lease CFG checks. This restricted compiler path
does **not** emit plain unlocked class storage, nor does it compile a
general-purpose field accessor. An unrelated owner, duplicate attached
schema, second inline schema, multi-field, bool/deferred state, alternate
callback body or unproved effect must reject without a guest artifact.

The required execution test `tests/stateful_method.mjs` asserts that the
attached and inline sources emit **byte-identical** native-compiled Wasm
and byte-identical wasm32-compiler output, and executes actual guest
`Counter.increment` with initial value 17, 25→42 checked update, overflow
rollback, exact lease release and instance isolation. The existing source
stateful receiver preview is deliberately *not* the full canonical Counter
program: it does not compile `on_start`, `receive(ActorMail<int>)`, a
real actor mailbox or the `using state.read/write` effects of the requested
example. No production `compile()` support is turned on.

**Evidence status:** tests have been authored for the latest source head.
The previous pinned private CI evidence applies only to the original
head `e6dff1d9c292a7243e21df888c84b5f43b8816f0`, not the amended PR
head. The latest source Actions attempt `44663206929` failed before
running any steps; real Rust/native/wasm32 test results for the new head
remain `NOT_EXECUTED`. Keep Draft pending exact-head CI and review.

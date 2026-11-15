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

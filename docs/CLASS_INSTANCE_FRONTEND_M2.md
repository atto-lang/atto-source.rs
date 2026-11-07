# M2 class-instance frontend, stage 1 — not executable objects

The compiler now **parses and preserves** class instance field syntax, e.g. `define class Counter as final int count: 0; let ready: bool = false; end`, plus the constructor expression `new Counter(10)` in its Rust AST. It records field names, nominal class identity, source positions, field type, final-slot intent, const-required intent, and initializer AST; duplicates, invalid types and constructor argument caps are checked.

**Execution remains explicitly blocked.** The checker returns `E_UNSUPPORTED` with no guest binary for any nonempty class instance layout, constructor expression, or class-valued callable signature. No object allocator, constructor invocation, per-object ownership, borrow checking, `self`, field reads/writes, or virtual method dispatch exists yet. In particular this is **not** an actor implementation: actor state remains sealed, `new Actor()` is invalid, and actual actor isolation/mailbox scheduling belongs to later Web Worker development.

Next independent gates before execution: class allocation domain and revocation/lifetime contract, constructor/initializer typing and constant evaluation, nominal/class Sendable restrictions, ownership guards for `rt borrow` / `rt take` / `rt share`, per-object layout, and full native/Wasm compiler -> executed guest conformance. No changes to compiler ABI v2 or guest ABI v1.

This PR is stacked on the reconciled top-level declaration PR #7 and **must not be used to replace the previously verified compiler binary** for public Pages.

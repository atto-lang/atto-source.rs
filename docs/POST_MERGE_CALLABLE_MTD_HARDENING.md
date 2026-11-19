# Post-merge callable and local class receiver hardening

Production baseline: [merged #147](https://github.com/atto-lang/atto-source.rs/pull/147), merge commit `d06f76c3c5567b66168365caed11150ed6c95a3d`.

This changes **no compiler lowering architecture**. We retain `lower_class_receivers` in `src/target_syntax.rs` as the only executable local class `mtd` path. In particular, we do *not* add the competing `link_executable_class_receivers` implementation from [#156](https://github.com/atto-lang/atto-source.rs/pull/156). The production parser, type checker and guest Wasm emitter still receive the source in ordinary class-method form.

## Security invariant and explicit subset

Only a **local concrete class**, without namespace/imports, actor declarations or mixed actor receivers, can execute out-of-line methods. An out-of-line method is currently an **immutable query receiver**: its class fields must have supported scalar types; it may read `self.field` and call a separately checked immutable class `mtd`. An attempted `self.field = ...`, `+=`, `++`, `--`, standalone `self` capture/return/argument, or call to a normal inline method with unverified mutation effects fails compilation before Wasm emission. A mutable `mtd (mut self: C)` requires an ownership-aware exclusive loan and remains unsupported. The class-token guard is deliberately conservative while richer borrow/effect IR is not yet executable.

**Actor receivers remain a different ABI** and must never get synchronous class dispatch. The class `mtd` gate is not evidence of actor supervision, CONTROL, state sealing, or Java/LLVM/Forge/.NET parity. Do not publish candidate Wasm to the site's pinned compiler from this PR.

## Executed acceptance set

The conformance corpus retains all five braced/Erlang function, routine, inline class and out-of-line `mtd` spellings. Native compiler and wasm32 compiler must generate byte-identical guest Wasm, and real guest execution must return **42** for the original callable forms, **20** for the focused `Counter.double_value` fixture, and **25** for the already-merged five-form class mtd example. Negative fixture failure must produce the exact expected error code and **no emitted guest module**. Chromium must also compile and execute `Counter.double_value` to **20** inside a Web Worker and reject field mutation with `E_OWNERSHIP`.

CI: `cargo test --locked`, native and wasm32 compiler builds, `ATTOC=target/release/attoc node tests/conformance.mjs`, `ATTOC=target/release/attoc node tests/callable_guest_execution.mjs`, receiver/actor probes and `node tests/browser-e2e.mjs`. When source Actions fail before admission, mirror the **full Git tree and every changed blob hash** into a temporary private carrier. Only exact-head executed green evidence certifies this PR.

## Next ABI work

New ownership-safe first-class `fnc(int) => int` values and captured lambdas require a separate closure environment ABI, capture-lifetime proof and runtime guest execution. This PR makes no claim that those are executable.

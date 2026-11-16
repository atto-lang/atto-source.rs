# AOT-first trace counters (prototype)

This module is a bounded, host-side profiling substrate for future
trace-guided optimization in the Rust/Cranelift backend. It does **not**
compile, emit or execute JIT code, patch a compiled function, execute guest
actors, or establish ownership/GC guarantees.

- **AotOnly:** constructs zero counter slots and recording is a no-op.
- **CountOnly:** constructs 1–4096 fixed site counters before recording.
- A recorded numeric site ID performs an atomic saturating increment, with
  no per-event heap allocation, guest references, captured values or locks.
- Only one thread observes the transition at a configurable hot threshold.
- Snapshot collection allocates host metadata and is explicitly separate
  from recording; never call it in a hard-RT recording path.
- Site indices are validated, and counters do not wrap on overflow.

Tracing must remain opt-in. It must never change the default closed-world
full-AOT object emission, nor imply that the Oreslang scalar compiler has
a full interpreter, JIT, actor runtime, or garbage collector.

## Required follow-on work

1. Assign stable, source-aware typed MIR site identifiers and emit
   instrumentation only after semantic/effect checks.
2. Add explicit runtime counters and AOT/JIT ABI compatibility tests.
3. Require opt-in executable-memory authority and target support; disable
   native JIT on browser CSP, freestanding or hard-RT targets by default.
4. For browser delivery, implement a real wasm32 codegen and runner path.
   A native .o is not a Wasm module.
5. Record and compare hot-site trace receipts, guest behavior, latency,
   allocation and pause budgets against Java and LLVM with exact SHA runs.

No feature-parity or JIT-performance claim is justified by this PR alone.

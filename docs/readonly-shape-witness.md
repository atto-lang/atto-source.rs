# Checked read-only shape witness: mainline integration stage

**Status:** Host-side proof preparation only. No guest expression lowering, AOT
execution, map runtime adapter, or backend parity has been established by this
module. The new tests must actually execute on the exact commit before any
PASS claim.

## What is implemented

The compiler now has a small, dependency-free, `shape_witness` module that
represents required open-row fields, typed literal/union subtyping, and a
read-only projection from verified source field metadata. The proof retains
the **actual** nominal source ID, layout epoch and scoped-loan ID. Every
fixed-layout access refers to the **actual source field offset**; the
nominal type's differently-laid-out namesake is never treated as the same
pointer/layout.

Only public, readable, effect-free, guaranteed-present fields may be
projected. Duplicates, missing fields, incompatible types, sealed actor
state, invalid loans, absent or forged map snapshots, overlapping offsets,
empty rows, deeply nested shapes, and unverified handles are rejected.
Union subtyping is read-covariant; it is never write authority.

For maps, the module produces a **planned** snapshot-key read only when an
external compiler/runtime source provides a nonzero snapshot ID and proven
key presence. This is *not* a runtime snapshot implementation. No map plan
may be emitted as guest code without an atomic lookup / stable loan backend.

## Remaining integration milestones

1. Feed *checked* `check::ClassInfo` field metadata into the witness
   constructor after type resolution. Never rely on source-supplied
   `SourceShape` descriptors as an authority.
2. Define an actual non-escaping borrowed `ShapeView` formal ABI with
   receiver identity + verified field-offset projection; preserve the
   lifetime across calls and reject transfer/copy/mutating projection.
3. Compile source `fnc f(p: {foo: string})` and `fnc f(~Point p)`
   with the same type AST, specialize or pass the verified descriptor,
   and **do not** reinterpret `Other*` as `Point*`.
4. Add cross-layout ordinary-class guest oracles and negative cases for
   private state, method effects, moves, borrow escaping, mutable union
   writes and stale epochs.
5. Add bounded record/struct projections, then real Map key-presence
   checking backed by a stable snapshot read. Reuse the same conformance
   registry on native Wasm, browser Wasm, Graal, LLVM, .NET and Forge.
6. Merge the still-diverged structural PR #187 only after semantically
   reconciling it with current main and running exact-head tests.

## Verifier commands

```bash
cargo test --lib shape_witness -- --nocapture
cargo test --test shape_witness_contract -- --nocapture
cargo test --lib
```

These tests are genuine Rust unit/integration checks when they run, not
`atto` guest execution or an actor-supervisor integration proof.

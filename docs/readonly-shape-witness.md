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

## 2026-11-20: checked structural *member* witness plan (non-emitting)

[atto-spec #36](https://github.com/atto-lang/atto-spec/pull/36) and
[atto-platform-conformance #119](https://github.com/atto-lang/atto-platform-conformance/pull/119)
now describe seven structural kinds (interface, trait, class, struct, Record,
Tuple, Map). The companion `src/structural_member_witness.rs` is a pure,
non-emitting compiler witness planner, exposed as
`atto_compiler::structural_member_witness`.

The planner accepts **checked compiler declaration metadata only** and
requires every exported field and instance method in a requested contract.
It checks method arity and parameter contravariance, result covariance,
receiver mode, argument/result ownership, async, effects, permission subsets,
generic bounds, public instance visibility and unique overload resolution.
Successful method planning records the real concrete source nominal ID,
concrete dispatch slot, full required callable signature, layout generation,
borrow/loan identity and **nonzero, equal TenantVM IDs**.

Field proofs reuse `shape_witness`'s actual-source offsets. Tuples require
exact positional arity/types, while Maps require exact key/value types and
an independently checked stable key-presence/snapshot witness for any named
field shape. Merely having `Map<string,int>` cannot prove the key `"foo"`
exists. No private/sealed actor state can be projected.

**Important nonclaims:** This stage does not connect a production
interface/trait AST to the witness builder, does not verify a concrete method
slot against the actual AOT/Wasm function table at invocation, does not
invoke guest methods, does not implement writable projections, and does not
provide a Map runtime snapshot adapter. Therefore it does **not** make
`fnc use(~Reader r) { r.read(); }` guest-executable. Existing unsupported
source must continue to fail closed. A source-only contract or standalone
Rust witness test is not backend parity.

### Additional verification

```sh
cargo test --test structural_member_witness_contract -- --nocapture
cargo test --test shape_witness_contract -- --nocapture
cargo test --lib
```

The 16-case Rust integration suite includes public/all-member compliance, effect and
permission negatives, borrow/async receiver safety, overload ambiguity,
cross-nominal field offsets, tuple exact arity, Map key-snapshot planning,
stale loan rejection, method-only malformed receiver layouts, duplicated concrete
method slots, and TenantVM mismatch. All results must be associated
with the **exact tested source SHA**. GitHub workflows failing with no job
steps are **NOT_EXECUTED**, not compiler test failures or PASS.

### Next codegen gate: checked method thunk specialization

Build a compiler-owned `StructuralMethodThunk` plan with the actual source
receiver type, selected method symbol/slot, exact lifetime/effects/ownership
proof and an AOT/Wasm-verifiable ABI signature. Lower only a bounded static
class implementation through a concrete receiver thunk (or checked static
specialization), never a pointer cast to the requested nominal interface.
Evaluate each receiver exactly once, prove its loan remains live, and reject
temporary/escaping/mutable/actor/dynamic calls until independently supported.
Then add **real executable Atto guest** positive/negative tests and align
all other backends independently.

## Marker/brand constraint preparation (candidate)

`src/structural_tags.rs` composes the existing member witness with a compiler-owned registry of scoped package/module/declaration IDs. Flat registered nominal types have checked marker/brand attachments; named targets require all target tags, and anonymous targets retain only explicitly requested tags. Public brands can be referenced but not attached by unrelated modules; public ordinary markers are attachable. Duplicate, stale, unknown and unauthorized metadata fail closed. Matching tags never waive member, tenant, layout or loan checks. The registry is compiler metadata, not guest payload storage, runtime UUID identity or proof of a value invariant.

The source and target metadata must come from the same checked resolver epoch and actual declarations; caller-supplied IDs are not guest input. Production parser, name resolution, checked construction/mutation, inheritance, target binding, cache invalidation and guest lowering are not yet integrated. Do not call the old member-only planner directly for tagged/named source contracts. `cargo test --lib structural_tags::tests -- --nocapture` runs nine focused host unit tests; exact-head execution remains unverified. See [spec #273](https://github.com/atto-lang/atto-spec/pull/273) and [guest fixtures #676](https://github.com/atto-lang/atto-platform-conformance/pull/676). No guest or backend parity claim.

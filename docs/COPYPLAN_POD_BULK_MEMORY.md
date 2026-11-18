# CopyPlan optimization: flat POD bulk-memory copy (Rust/Wasm trial)

**Status: stacked draft PR #337; current hardening needs new exact-head CI evidence.**
Dependency: atto-source.rs #265; canonical closed-world type copy witness from
atto-borrow-checker.rs #32. This is an optimization of already-admitted
`copy!!` semantics, not a new source operator.

## Proof preconditions

The verified concrete class must (1) have a complete known field inventory,
(2) be an ordinary class, never protected actor state, (3) have no custom
`copy_deep` override, (4) contain at least one stored field and (5) consist
**exclusively of inline `int`, `bool`, `float` or enum slots**.
All of these fields have identity-free, fixed-size layout in this restricted
Wasm guest ABI, with contiguous `FIELD_BYTES` slots.

`authenticate_flat_copy_aot` runs the canonical descriptor checker before
generating a copy function, and again at the `copy!!` call. This optimization
only selects a different body *after that gate*. It must never mint a new
Copyable/DeepCopyable witness, infer Sendable from Copyable, or turn `copy!!`
into `share!` or `take!`.

## Lowering

The generated function still receives one source object, allocates a **fresh**
destination object, and returns it. For the verified POD profile it emits one
typed-IR `FlatPodCopy` lowered to WebAssembly
`memory.copy(dest, source, field_count * FIELD_BYTES)`. Both source and
destination are compiler-created local expressions evaluated once. The source
is not moved; the copy has a distinct mutable identity. The bulk instruction
does not call constructors, traverse runtime reflection tables, or introduce
new actor-message semantics.

Classes with strings continue using individual typed field loads/stores.
Strings are *not* classified as POD even though the current guest stores a
packed string value. Nested class handles, arrays, actor/resource/borrowed
fields and unresolved or polymorphic type graphs remain rejected before
codegen. No byte-copy is permitted for those representations.

This optimization can reduce instruction count, but is **not** automatically
faster for every object size. Benchmark over varying field counts, warm and
cold runs, production targets, allocation patterns and real guest execution.
Do not assert speedups based solely on Wasm instruction count.

## Required evidence

- Rust compiler tests: POD path emits the bulk-memory opcode, string-bearing
  flat class does not, and deterministic compile output remains unchanged.
- Executed guest fixtures: mutation isolation, separate copies per recipient,
  constructor side-effect avoidance and mixed int/float/bool field values.
- Negative gates: unverified user overrides, active mutable loans and nested
  arrays still reject with their established diagnostics and no guest artifact.
- Source-exact CI: pinned Rust toolchain, `cargo test --locked --all-targets`,
  wasm32 compiler build and Node WebAssembly guest executions. Earlier
  source/pr #265 evidence does not automatically validate this new branch.

## Explicit non-goals

No nested graph cloning, actor mailbox fan-out, zero-copy arena transfer,
persistent collections, uniqueness-based in-place update, or backend parity is
claimed here. Those are separate follow-ups with their own proof obligations.

## Hardening audit (2026-11-18)

The generated bulk-copy instruction carries only two compiler-verified local
indices and a checked byte count. It does not carry source expression trees,
so a future emitter cannot accidentally re-evaluate a side-effecting address.
Both the class constructor and clone size use checked multiplication and
conversion bounded by the fixed 16-page Wasm heap; codegen independently
asserts a nonzero whole-slot in-bounds size even in optimized builds.

The Rust tests inspect the generated typed IR to verify the exact 24-byte
source/destination local-slot plan and to require typed field-store fallback
for a class containing a string. The Node guest fixture also checks raw
WebAssembly memory after mutation: the original object retains int=42,
float=1.5, bool=true while the independent clone has int=7, float=1.5,
bool=false. This is in addition to the existing guest return and negative
diagnostic checks. No performance claim or generalized graph admission.

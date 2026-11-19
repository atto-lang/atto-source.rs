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

## Backend boundary revalidation

The Wasm emitter independently verifies the exact shape of the generated
`<Class>.copy_deep$generated` body before emitting `memory.copy`:
one checked allocation in local 1, one 0-to-1 bulk copy whose byte count
matches the allocation, and one return of that same local. Input, result,
allocation and local types must all match the concrete class type, and the
copy size must be a nonzero aligned number of bytes within the fixed heap.

Raw-memory instructions in arbitrary user functions, actors, nested control
flow, or malformed typed-IR functions cause a compiler-side assertion rather
than emitting an unproved Wasm memory access. This is defense in depth: no
user-supplied source annotation can create an AOT witness.

A Rust adversarial test mutates a valid compiler-generated typed program in
seven ways (wrong source/destination slots, wrong byte count, spoofed function
name, missing destination local, extra instruction, and an unauthorized copy
in main) and requires every invalid variant to fail before producing Wasm.
These are compiler rejection checks, not an authorization mechanism for
untrusted external typed IR. All cross-backend portability remains separate.

## Merge-admission follow-up (2026-11-18)

The end-to-end fixture `999-copy-doublebang-pod-interleaved-snapshots.atto`
creates an original, a first clone, an unrelated same-layout allocation,
and a second clone. It mutates both clones differently, verifies source and
unrelated object isolation, and inspects all **four distinct 24-byte objects**
in guest Wasm linear memory. This catches copying from the wrong source,
using the wrong destination, omitted fields, and accidental aliasing.

`scripts/ci/copyplan_admission.sh` is a portable no-secrets admission entry
point for exact mirrored source trees and a normal source checkout. It runs
the pinned full Cargo suite, native and Wasm32 compiler builds, compiled
guest execution and full conformance. It also invokes
`scripts/ci/guard_copyplan_clippy.py`: fail on previously unseen Clippy
diagnostics, while reporting the 13 known pre-existing warnings as **debt**.
The guard is not a substitute for eventual whole-repository rustfmt/Clippy
cleanup. The SHA-pinned vendored `src/copy_graph_v1.rs` remains unchanged.

The carrier workflow must authenticate the **complete original Git tree**
and its blob count *before* invoking the admission script. The test carrier
cannot use a private source checkout with its own `GITHUB_TOKEN`; the
source tree must be mirrored into a temporary branch through a trusted Git
connector, with exact Git object SHA verification for changed blobs and the
full tree. Post the exact source head, tree, carrier head and execution
receipt on both stacked PRs. Do not mark draft PRs ready/merge solely on
green carrier tests: #265 must be independently reviewed first, with
current-head tests, and #337 must subsequently be rebased/retargeted.

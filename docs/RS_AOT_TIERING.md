# Rust AOT-first optimization policy (stage 0)

This repository compiles a narrow scalar subset with Logos, independently validated
Rust MIR and Cranelift **native object emission**. It does not yet implement a
general actor runtime, JIT engine, WebAssembly emitter, mobile or desktop linker,
or a zero-GC general-purpose heap. A successful policy decision is **not**
proof that the selected deployment target can compile or execute.

## Policy and layout API

`src/tiering.rs` introduces pure Rust primitives with no third-party runtime
dependency and no hidden dynamic allocation on trace observations:

- `ExecutionPlan::admit`: always retains a required AOT baseline. The
  `AdaptiveJit` request is allowed only for trusted native server/desktop
  hosts that explicitly permit executable memory and only in dev/release.
  It is forbidden in strict-AOT, hard-RT, browser Wasm, WASI and embedded
  targets. No executable memory is allocated and no machine code is JITted.
- `TraceCounters<N>`: stack/inline fixed-capacity counters with checked
  slot indices, saturating increments, a nonzero threshold and exactly one
  `BecameHot` event per slot. The event is an optimization *recommendation*,
  not a runtime request to patch a code pointer.
- `recommend_layout`: a deterministic, evidence-based next-build layout
  choice for AoS, SoA or AoSoA, with checked power-of-two SIMD lanes and
  an explicit inverted-class hint. A physical layout is chosen once;
  no live data is relocated or mirrored by this API.

## Target contract

| Target | Stage-0 AOT-first policy | Runtime JIT policy | Implemented guest compiler |
|---|---|---|---|
| Native server | Required | Trusted host only, dev/release | Scalar Cranelift object only |
| Native desktop | Required | Trusted host only, dev/release | No desktop packaging/runtime |
| Browser Wasm | Required | Denied | No Wasm emitter in current main |
| WASI | Required | Denied | No WASI object/link pipeline |
| Embedded | Required | Denied | No embedded guest/runtime |

Offline profile-guided AOT is permitted as a policy for each target; collection,
serialization, replay and reproducible profile provenance are not yet implemented.

## Mandatory implementation gates before enabling guest features

1. Typecheck `define inverted class` and physical layout selection in owned HIR/MIR,
   including stable nominal identity and field-sensitive borrowing.
2. Introduce actor-local nonmoving arenas, checked generations/domains and frozen
   immutable shares; do not cast a `CompressedRef` into a dereferenceable pointer.
3. Integrate bounded trace counters at verified MIR safepoints; preserve scheduler
   reduction budgets, cancellation, exception and actor-control boundaries.
4. Only enable actual JIT after signed/verified code provenance, W^X memory
   discipline, target permission proofs, deoptimization maps and executable tests.
   Browser and hard-RT variants must remain independent of JIT.
5. Prove native/browser/desktop semantics with exact-source Rust integration,
   object-link-execute tests and Java/LLVM differential fixtures. Never infer
   execution support from these admission-policy unit tests.

## Rust test gate

```sh
cargo fmt --all -- --check
cargo test --locked --all-targets
cargo clippy --locked --all-targets -- -D warnings
```

Tests for this policy live in `tests/tiering_policy.rs`. Use Rust tooling only:
do not introduce Python tests or scripts.

# Checked i64: constant-admission versus guest runtime traps

**Resolution:** AttoLang uses checked signed-64-bit arithmetic by default.
This document reconciles the incompatible arithmetic policies proposed in
[`oreslang-source.rs#37`](https://github.com/ores-truffle-oreslang/oreslang-source.rs/pull/37)
and [`oreslang-source.rs#38`](https://github.com/ores-truffle-oreslang/oreslang-source.rs/pull/38).
It does **not** enable the former backend's legacy default wrapping mode.

| Source context | Provable signed-i64 overflow | Runtime-dependent overflow |
| --- | --- | --- |
| Explicit local `const` | `E_CONST_OVERFLOW`; no guest artifact | Not certified as constexpr by this narrow pass |
| Explicit class-field `const` | `E_CONST_OVERFLOW`; no guest artifact | Existing `E_CONST` admission applies to effectful initializers |
| `val`, `let`, `final`, ordinary `return`, call body | Compile checked Wasm; overflow traps with `panic_code=1` | Compile checked Wasm; overflow traps with `panic_code=1` |

## Compiler contract

The checker first resolves and validates types, then walks **only explicitly
constant** typed expressions. It evaluates known integer and boolean operations
with Rust `checked_add`, `checked_sub`, `checked_mul` and `checked_neg`
without rewriting or deleting the checked Wasm guest computation. Proven
constant locals are tracked by typed slot and lexical scope. All operands of
eager binary operations are inspected before the parent can be called safe;
`(MAX + 1) * 0` is therefore not silently folded to zero. For a ternary
whose predicate is provably Boolean, only the selected arm is evaluated;
the checker has separately typechecked both arms. This matches runtime
short-circuiting without introducing a new eager overflow failure.

The classifier returns `None` for expressions that it cannot prove constant,
not a success certificate. It does not claim to implement the full typed
constexpr evaluator or arbitrary constant purity. In particular, existing
compatibility handling of some local `const` calls remains outside this
focused change. The separate broader
[`atto-source.rs#102`](https://github.com/atto-lang/atto-source.rs/pull/102)
should be reconciled before enforcing full constexpr purity.

No checked-`i64` traps become recoverable `throw` effects; the fatal guest
overflow boundary remains distinct from actor exception semantics. This PR
does not publish conditional-trap-complete effect IR, modify TenantVM
capabilities, introduce executable imports, or claim other backends' parity.

## Evidence and nonduplication

- `tests/checked_const_overflow_boundary.rs` checks source-level diagnostic
  categories, alias tracking, signed MIN/MAX, class `const` versus `final`,
  chosen-arm laziness, and ordinary source admission.
- `tests/checked-const-overflow-guest.mjs` compiles real guest Wasm with the
  native `attoc`, rejects output on constant failure, and asserts actual
  `WebAssembly.RuntimeError` plus guest `panic_code=1` for otherwise admitted
  runtime overflow. Missing compiler or guest compilation is a failure.
- The broad 21-case scalar-v4 parity/byte-identity corpus belongs to
  [`atto-source.rs#399`](https://github.com/atto-lang/atto-source.rs/pull/399)
  and is not copied or weakened here.

**Release gate:** exact-head `cargo fmt --all -- --check`,
`cargo clippy --locked --all-targets -- -D warnings`,
`cargo test --locked` and the new executable Node guest test must pass on a
runner with nonempty steps and independently reviewed source provenance.
If cross-org private-source validation is needed, it must be authorized under
current source-export policy; source ownership and per-blob integrity must be
verified. A zero-step workflow is **NOT_EXECUTED**, never a passing test.

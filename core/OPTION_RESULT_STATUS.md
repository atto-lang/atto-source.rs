# atto-lang/atto-source.rs — native Atto core Option/Result source candidate

These `.atto` modules are intentionally owned by this compiler and may
diverge in implementation from Java, Rust and other backends. Public semantics,
not shared source blobs, are the intended parity boundary.

**State: SOURCE_CANDIDATE; compiler admission and guest execution NOT_VERIFIED.**
Both files declare file-scoped `use namespace core;` and an explicitly public
module. A namespace is not an ambient import: callers still require a verified,
explicit module edge. `tests/core_option_result_namespace.rs` checks lexical
namespace identity and denies duplicate/late declarations; it does **not**
validate generic runtime support.

The files under `core` are *not* automatically linked into the compiler's
standard library. Nominal type constructors and runtime representation remain
compiler primitives. These files must not create a host-language shim.

Supported public semantic targets:
- `Option<T>`: Some(T)/None, `flatten`, `map`, `and_then`,
  `or_else`, eager `to_result`, `transpose`.
- `Result<T,E>`: Ok(T)/Err(E), `flatten`, `map`, `and_then`,
  `map_err`, `or_else`, `to_option`, `transpose`.
- Exactly-once callback on the matching tag; never invoke on the other tag.
- `None` and `Err` remain ordinary values; `?` may only propagate
  compatible outer carriers and must run cleanup; callback throw/raise/panic
  retain their ordinary effect semantics.
- Every affine active payload moves once; inactive payload must not be read;
  moved values cannot be reused; drop exactly once on both success/failure.

**Required evidence before marking supported:** actual syntax/typechecking of
these .atto files, import and module resolution, link against correct nominal
ADT ABI, positive and negative compiled guest cases, active-payload ownership
and callback-effect tests, AOT compilation, current-head stepful CI receipts.
The four `.atto` fixtures only specify expected behavior; simply adding them
does NOT assert they execute or that full generic functions are admitted.

The high-level callback helpers are an API target, not blanket permission
for non-Copy/affine callback capture. Reject unsupported generic/effect shapes
until the local compiler proves their safety.

Backend note: Rust/Wasm compiler: preserve existing parser and Wasm emission. Typed generics and Fnc callback carriers remain gated until real compiled tests.

### Additional parity vectors
- `tests/core_option_result/tag_zero_42.atto`: expected 42; distinguish
  Some(0) from None and Ok(0) from Err(0) using nominal tags.
- `tests/core_option_result/negative_nonexhaustive.atto`: must reject a
  partial Option match with no guest artifact.

Rust wires scalar fixtures into a compiler unit test that requests real Node
Wasm execution, with explicit guest result checks. That test is committed but
has no observed stepful exact-head CI receipt; other backends require their
own executable source-level admission.

## Borrow-only source API & affine ownership admission (2026-11-22)

This backend-owned draft now includes `option.is_some_borrowed<T>`,
`option.is_none_borrowed<T>`, `result.is_ok_borrowed<T,E>` and
`result.is_err_borrowed<T,E>`. Each formal is `borrow Option<T>` or
`borrow Result<T,E>` and may only inspect the nominal active tag. This
**does not authorize extraction, moving or dropping a payload** from a
loan. These helpers are source **candidates**, not compiler-verified
admitted APIs; a backend without typed generic borrowed formals must
reject rather than erase ownership.

The new fixture `tests/core_option_result/borrowed_read_42.atto`
requires the real compiler to admit two read-only loans, preserve the
original owners for subsequent `unwrap` and execute an actual guest that
returns `42`. The separate
`tests/core_option_result/negative_affine_borrow_escape.atto`
requires a *typed ownership* rejection of borrowed `Option<Payload>`
`unwrap`; a syntax error, unsupported class construct or generic parse
failure alone must **NOT** count as a borrow-checker PASS. No guest
artifact may be emitted for this negative case.

Remaining proof obligations: no loan-to-owner promotion through
`flatten`, `map`, `and_then`, `transpose`, `or_else` or callback;
exactly-once destructor of each consumed affine active variant;
loan lifetimes through match arms, exceptions, suspension and actor
handoff; and separate AOT/sanitizer evidence where applicable.
The source-local CI runner has not supplied a stepful compiled guest
receipt for these new cases. Private compiler-source export requires an
independently approved exact snapshot and destination and must not
be inferred from a token-free mirroring technique.

### Real Rust compiler/guest test entrypoints for borrow queries

`tests/core_option_result_scalar.rs` now contains an actual Rust compiler
`compile(...)` call for the new `borrowed_read_42.atto` target, and a
Node `WebAssembly.instantiate` guest execution assertion for returned
`42` with no benign host simulation fallback. A separate test compiles
`negative_affine_borrow_escape.atto` and requires a typed borrow/ownership
diagnostic (`E_OWNERSHIP`, `E_BORROW` or `E_MOVED`) with source position,
**not** a lexical or syntax error. The test code is executable and not
ignored. Both cases are **NOT_EXECUTED** on an admitted exact-head runner.
Until a full real compiler run proves the feature, this branch remains
draft; existing generic borrow classes or namespace linkage may fail.
Do not downgrade the tests to host-only success to make a CI badge green.

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

### Bounded concrete ADT borrow checker extension — 2026-11-22

The actual Rust compiler's `src/check.rs` now admits explicit
`borrow! named_owner` *at a synchronous call site* for only the
already-specialized, immutable guest types `Option<int>` and
`Result<int,string>`. It evaluates the owner exactly once and checks
the original local's moved/exclusive-loan state. This narrow shared
read does not expose a writable payload, create a new non-Copy owner,
or extend the owner's lifetime. A temporary ADT, mutable ADT borrow,
generic `Option<Payload>`, and escaped affine class loan remain
rejected; no unsupported type is silently upgraded to an owned ABI.

The executable Rust test `borrowed_option_result_tags_execute_wasm_and_preserve_copy_owners`
now uses only the browser compiler's admitted concrete ADT types and
requires **real WebAssembly guest execution returning 42**, then
matches both original carrier values after the read calls. The
`borrowed_affine_payload_escape_requires_ownership_diagnostic` test
now checks an actually reachable class-owner `E_BORROW`, rather than
mistaking the unsupported `Option<Payload>` parser error for semantic
proof. Additional tests independently require
`E_UNSUPPORTED_TYPE` for `Option<Payload>` and `E_BORROW` for
temporary/mutable ADT loans.

**Evidence classification: AUTHORED / NOT_EXECUTED on a current
compiler runner.** The generic source `core/option.atto` and
`core/result.atto` borrowed formals, consuming helpers, import
linking, general affine ADT payload layouts and AOT cleanup are still
unverified. Do not promote these until each has its own compiled
positive and typed-negative receipt. A source-only Node metadata
suite does not certify the compiled Wasm behavior.

### Correction: concrete Option/Result borrow marker is Copy, not a loan

A follow-up audit removed the special `args()` path that had accepted
`borrow! Option<int>` / `borrow! Result<int,string>` without a
borrow-qualified formal-parameter mode. That bypass is not permitted.
Instead, these two current concrete tagged-sum profiles are now explicitly
classified as Copy because their payloads (int and immutable string) are
Copy. Their normal value-consumption path therefore preserves the source
local, and `borrow! owner` on these types is only a non-destructive Copy
projection. It does **not** establish a lifetime-bearing loan, nor may it
be used as evidence for generic `borrow Option<T>` / `borrow Result<T,E>`
formals or affine payload behavior.

The class-owner `E_BORROW` negative still exercises the existing lexical
loan checker; an attempted return of `borrow! owner` for a non-Copy
class must be rejected. `Option<Payload>` remains
`E_UNSUPPORTED_TYPE`. Temporary and mutable ADT borrow attempts are
still expected to reject with `E_BORROW`. The test method/fixture comments
now distinguish Copy-carrier reuse from real class-loan semantics.

This distinction is deliberately conservative: **the Rust browser compiler
does not yet implement general borrowed Option/Result parameters**. The
positive Copy-carrier guest and class-loan negative must both execute on the
exact source head before any claim is promoted.

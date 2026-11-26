# Compiler-verified units of measurement v1

Status: **draft, closed algebra plus candidate source frontend; execution pending**. Feature 11 of the 15 proposals.

Related, not duplicated: atto-spec #209 owns numeric *value* semantics
(checked ints, decimal rounding, compile-time dimensions in DOP kernels).
atto-spec #204 / rs #514 own opaque bounded newtypes. This slice is the
unit layer that sits on top of both.

## Problem

`timeout_ms + elapsed_s`, `buffer_len_bytes > distance_m`, and
`KB` vs `KiB` are real production bugs that plain `int` cannot catch.

## Algebra (`src/units.rs`)

* A unit is an **exact rational scale** (u128/u128, reduced) times integer
  powers of nine base dimensions: `m kg s A K mol cd` plus `B` (information)
  and `count`. Bytes and counts are dimensions because they are the most
  commonly confused "plain ints".
* `+ - == < <= > >=` and assignment need the **same unit**. Same dimension at
  a different scale gives `E_UNIT_SCALE_MISMATCH` (no silent `s + ms` rescale).
  A different dimension gives `E_UNIT_MISMATCH`.
* `* /` compose dimensions and multiply scales exactly (`kg*m/s^2 == N`).
* `conversion_factor(from, to)` is exact (`km/h -> m/s = 5/18`,
  `MB -> MiB = 15625/16384`). `convert_int_exact` requires the converted value to be integral, not the
  conversion factor. 1000 ms converts to 1 s; 1500 ms gives `E_UNIT_LOSSY`. Overflow gives
  `E_UNIT_OVERFLOW`.
* The unit expression grammar is `term (('*'|'/') term)*`, with
  `term = name('^' -?digits)? | 1`. Exponents are bounded to ±12. Parentheses
  are rejected in v1 rather than guessed.

## Superseded experimental surface (not parsed; do not implement as canonical)

```atto
val int[ms] timeout = 250 of ms;
val int[s]  elapsed = 2 of s;
val int[ms] total = timeout + convert(elapsed, ms);   // ok: exact factor 1000
val int[ms] bad   = timeout + elapsed;                // E_UNIT_SCALE_MISMATCH
val int[B]  wrong = timeout;                          // E_UNIT_MISMATCH
fnc speed(int[m] d, int[s] t): float[m/s] { return d / t; }
```

The candidate canonical surface is [atto-spec #224](https://github.com/atto-lang/atto-spec/pull/224), with `define dimension`, `define unit`, contiguous `5000<Millis>` literals and `.to_unit<Second>()`. The syntax below/above is historical only. Units may be erased only after checking all boundaries: `int[ms]` lowers to a plain i64, so there is
zero runtime cost and the same Wasm, LLVM and JVM layout. Exact conversion requires divisibility and checked range; a single multiply is not sufficient.

## Candidate source preview

```atto
define dimension Time;
define unit Second: Time;
define unit Millis: Time = Second / 1000;
pub fnc main() -> int {
    return 84000<Millis>.to_unit<Second>() / 2<Second>;
}
```

The new frontend consumes a complete standalone source file, resolves file-local nominal dimensions and exact rational scales, checks every closed arithmetic operation, and passes only a proved successful dimensionless result to the existing scalar Wasm compiler. It is deliberately narrower than spec #224: 256-bit reduced scale components with 576-bit conversion work buffers, no imports, no general quantity bindings/signatures, no affine values or polymorphism, no dynamic quantity ABI. The closed frontend now retains typed arithmetic failures through the normal Wasm emitter: overflow sets panic_code 1 and division by zero sets panic_code 2, then traps. The entire source must type-check before a trap artifact is emitted. This is not general runtime quantity arithmetic. The older nine-dimension algebra remains a separate internal API. The normal CLI path and direct compiler entry share this preview; imported quantities fail closed.

## Tests

`tests/units_of_measure.rs` (9 cases): scale mismatch, cross-dimension
rejection, derived-unit identities, exact conversion factors, exact or lossy
integer conversion with overflow, canonical dimension display, and
malformed/unknown expressions.

## Next slices

1. Run the focused exact-head quantity test workflow and actual Wasm guest; preserve all origin gates.
2. Verify the new 256-bit scale arithmetic and the closed-profile trap lowering at the exact head.
3. Use compiler-resolved package/module/declaration IDs for imported and compound dimensions.
4. Integrate checked quantity bindings, signatures, FFI and serialization; add affine and polymorphic profiles only after separate specification review.
5. Execute shared conformance #584 with comparable backend receipts.

The legacy immutable-binding Cranelift tests are retained verbatim in `tests/legacy/immutable_binding_parity_cranelift.rs`; the browser crate now has its own real compiler regression in `tests/immutable_binding_parity.rs`. The original Cranelift/effect-IR suite is not claimed as executed by this change.

Wave 4 adds a dependency-free fixed-width rational implementation for the nominal frontend and focused source tests for wide scales, exact conversions and malformed decimal/quantity tokens. The legacy SI algebra retains its own 128-bit limits; its API is not the nominal declaration profile. The new shared corpus has 119 cases. These changes remain pending Rust compiler execution and cross-backend guest receipts.

Wave 5 separates static rejection from closed checked arithmetic failures. The frontend carries the first eager left-to-right arithmetic failure separately from the unit type, checks subsequent syntax and static unit obligations, and selects the existing Wasm panic ABI only on a well-typed complete program. Focused CI tests and the new Rust shared-corpus driver in conformance #584 remain pending real compiler execution. The merged specification's current profile still excludes quantity variables, signatures, dynamic conversions, imported identity and general effects; those require a separate design and compiler integration.

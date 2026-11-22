# Compiler-verified units of measurement v1

Status: **draft, first executable slice** (Rust compiler library: the unit
algebra the checker will call). Feature 11 of the 15 proposals.

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
  `MB -> MiB = 15625/16384`). `convert_int_exact` allows only integral
  factors (`s -> ms`). Downscaling (`ms -> s`) gives `E_UNIT_LOSSY` until the
  caller picks a rounding mode from the numeric contract. Overflow gives
  `E_UNIT_OVERFLOW`.
* The unit expression grammar is `term (('*'|'/') term)*`, with
  `term = name('^' -?digits)? | 1`. Exponents are bounded to ±12. Parentheses
  are rejected in v1 rather than guessed.

## Proposed surface syntax (for spec review; not parsed yet)

```atto
val int[ms] timeout = 250 of ms;
val int[s]  elapsed = 2 of s;
val int[ms] total = timeout + convert(elapsed, ms);   // ok: exact factor 1000
val int[ms] bad   = timeout + elapsed;                // E_UNIT_SCALE_MISMATCH
val int[B]  wrong = timeout;                          // E_UNIT_MISMATCH
fnc speed(int[m] d, int[s] t): float[m/s] { return d / t; }
```

Units are erased after checking: `int[ms]` lowers to a plain i64, so there is
zero runtime cost and the same Wasm, LLVM and JVM layout. `convert` lowers to
one checked multiply.

## Tests

`tests/units_of_measure.rs` (7 cases): scale mismatch, cross-dimension
rejection, derived-unit identities, exact conversion factors, exact or lossy
integer conversion with overflow, canonical dimension display, and
malformed/unknown expressions.

## Next slices

1. Parse `T[unit]` annotations and `of unit` literals into the typed AST,
   then call `check_same`, `mul` and `div` from the checker. Erase before codegen.
2. `define unit furlong = 201168/1000 m;` user units (bounded registry).
3. Affine units (°C/°F) as a separate, explicitly offset-aware kind.
4. Conformance fixtures shared with Java/LLVM through atto-platform-conformance.

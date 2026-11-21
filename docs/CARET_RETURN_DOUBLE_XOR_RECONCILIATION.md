# Canonical caret and XOR spelling — current-main Rust/Wasm reconciliation

Candidate language authority: `atto-lang/atto-spec#143` (still a draft).
This patch updates the existing `atto-source.rs#411` rather than
creating a duplicate of historical bitwise PR #334 or discarding the
already-merged implementation from #382.

## Syntax boundary

- At statement start, `^ value;` is exactly `return value;`.
- At expression position, contiguous `int ^^ int` is integer bitwise XOR,
  using the existing checked type and Wasm i64 bitwise lowering.
- An infix single caret `int ^ int` is invalid; neither exponentiation nor
  integer XOR is implied by `^`.
- A spaced `^ ^` sequence does not form XOR; the compiler demands the
  two carets share one token span (adjacent source columns).
- The old `^=` compound spelling is rejected. `^^=` is **not introduced**;
  the language spec must explicitly approve any future compound shorthand.
  Write `value = value ^^ mask;` for checked mutable locals.
- Existing `<<<`, `>>>` shift and typed `<<`, `>>` stream/channel
  admission paths remain separate, not overloaded.
- `Math.pow` is the exponentiation API spelling, not a caret operator.
  This change does not establish its library implementation.

The lexer preserves single-caret return tokens and now permits adjacent
pairs; the parser combines a pair only in binary XOR position. This
avoids a new token variant or a hidden caret-return meaning change.

## Test and migration policy

Current branch tests migrate actual bitwise fixture source and native/wasm32
guest compilers from old `^` to `^^`; retain `^ match` return tests.
Negative cases prove that infix single caret, spaced caret pairs, and
retired `^=` compile to **no guest artifact**. Java currently uses
`^^` as Boolean XOR, **contradicting** canonical signed-integer XOR,
so passing Rust tests alone cannot establish Java parity.

CI uses exact-source checkout, toolchain Rust 1.85.1, `cargo test --locked
--test bitwise_ops --test float_division_ops --test fixtures`, real native `attoc`, generated
Wasm guest execution and no-artifact rejects, then a wasm32-hosted
compiler/guest execution and byte-for-byte native guest comparison.

Existing PR #411's former private runner evidence at
`51e10b711882284477714af1da56ee9f0e35c504` covers **obsolete**
source and does not prove this migration. The changed head must be
retested; absent or zero-step runs are `NOT_EXECUTED`. Full repository
CI, independent review, security, formatter, spec and cross-backend
conformance remain separate merge gates. Keep DRAFT.

## Checked shift-count and integer-division alignment — 2026-11-20

The approved target has **checked** shift distances 0..63 and `int / int -> float` ([atto-spec #151](https://github.com/atto-lang/atto-spec/pull/151)), superseding historical `n & 63` WebAssembly shift behavior and the former Rust checker diagnostic saying integer division policy was undecided. This existing draft now carries the corresponding source compiler work:

- `src/check.rs` types `int/int` division as `float` by reusing the already-implemented `TExpr::IntToFloat` path for **both** signed operands, retaining checked integer addition/subtraction/multiplication in int contexts. Returning that `float` from an `int`-declared function or assigning it to an `int` slot fails with `E_TYPE` before an artifact is emitted. The checked result's floating guest math is performed in Wasm `f64.div`, not integer truncation.
- `src/check.rs` rejects provably literal negative or >63 shift counts with `E_SHIFT_DISTANCE`; a nonliteral distance remains a dynamic checked obligation.
- `src/wasm.rs` passes the evaluated right-hand shift operand through a private, no-import `H_SHIFT_COUNT` helper before any `i64.shl`, `i64.shr_s`, or `i64.shr_u` opcode. It raises terminal `panic_code=7` on values outside 0..63, **without** Wasm's implicit modulo-64 truncation. Operand evaluation remains left-to-right, once each.
- Existing class/string runtime helper function indices are shifted consistently to allocate the new always-present helper; this index change, guest ABI and full regression suite require **actual native and wasm32 compiler builds** before being considered verified.
- Positive source fixtures `float-int-division.atto` and Rust/native/wasm32 test cases check typed floating 2.5, signed negative division, canonical XOR and valid shift bound 63. Negative no-artifact sources reject invalid shifts and float-to-int narrowing. Browser-compiler Node tests require actual wasm32-hosted compilation and byte-identical native guest modules. Dynamic invalid shifts must trap with the new panic code, never return an integer masked value.

**Current status: authored changes, NOT_EXECUTED** for real Rust Cargo, native `attoc`, wasm32 compiler, and generated guest runtimes on this branch's final SHA. No source transfer to an unapproved CI carrier; zero-step Actions failures do not count. This work does not implement generic `Result<T,Error>` trap, `Integer.div_trunc`, the final floating-zero-divisor/precision policy, or full multi-backend parity.

The superseded [Rust #334](https://github.com/atto-lang/atto-source.rs/pull/334) describes int XOR as single `^` and shift counts as masked; **do not merge its conflicting parser/shift implementation**. Preserve unique tests only after reconciling them into #411. The published [playground /play](https://github.com/atto-lang/atto-lang.github.io/tree/main/play) uses an older receipted Rust/Wasm compiler artifact; authored code here does not update it. The site's [draft #198](https://github.com/atto-lang/atto-lang.github.io/pull/198) must remain preview-only until an independently built, validated, same-source compiler binary is promoted.

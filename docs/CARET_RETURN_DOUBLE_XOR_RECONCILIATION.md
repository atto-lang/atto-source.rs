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
--test bitwise_ops --test fixtures`, real native `attoc`, generated
Wasm guest execution and no-artifact rejects, then a wasm32-hosted
compiler/guest execution and byte-for-byte native guest comparison.

Existing PR #411's former private runner evidence at
`51e10b711882284477714af1da56ee9f0e35c504` covers **obsolete**
source and does not prove this migration. The changed head must be
retested; absent or zero-step runs are `NOT_EXECUTED`. Full repository
CI, independent review, security, formatter, spec and cross-backend
conformance remain separate merge gates. Keep DRAFT.

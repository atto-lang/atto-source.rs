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
- The old `^=` compound spelling is rejected. Contiguous `^^=` is admitted
  for writable integer locals under the current bitwise-and-stream-operators-v1
  contract (atto-spec blob 1cd08cbae07da5788735b50d3f1f790ad04a9c48).
  Spaced caret/assignment tokens, immutable locals and non-integer operands
  remain rejected. Native and wasm32 guest tests are authored; execution pending.
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

## Numeric reconciliation — 2026-11-25

Spec chapter 17 (decided 2026-11-23) supersedes this draft's earlier int/int-to-float proposal. Integer quotient truncates toward zero; explicit floating operands preserve the floating path. The typed checker now keeps integer operands as integer. The Wasm runtime helpers guard zero and signed minimum divided by minus one. Named Integer.rem and Integer.mod are typed signed-i64 operations, and raw percent syntax remains outside this change.

The current bounded helpers report terminal panic_code 1 for overflow and 2 for zero divisor. This is **provisional**, not the specification's ordinary typed arithmetic Error ABI, and does not establish trap/Result parity. Helper allocation shifts class/string/source function indices, requiring a full compiler build and genuine guest regressions. The checked shift count remains unchanged.

Local Node 22 executed 189 independently reconstructed Wasm instruction probes (63 per quotient/remainder/modulus), including signed extrema and zero-divisor fault codes. No Rust compiler was invoked in this instruction-level test. The browser compiler guest suite now contains generated quotient, remainder and modulus cases; Cargo/native/wasm32 compiler execution on this PR head is still NOT_EXECUTED.

Preserve the superseded historical branch evidence separately; do not merge a competing integer-to-float implementation. The playground's published artifact is not updated by this PR. Do not promote without exact-head stepful compiler/guest verification and overlap review.

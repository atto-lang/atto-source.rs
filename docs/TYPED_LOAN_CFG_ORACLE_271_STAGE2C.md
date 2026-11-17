# Atto typed-loan CFG oracle — Stage 2c (issue #271)

**Status: experimental, diagnostic-only, branch-profile oracle. It MUST NOT
authorize codegen or relax an existing AttoLang ownership diagnostic.**

This draft is stacked on Atto compiler PR #290, which preserved compiler
loan IDs in actual `TExpr::LoanRead` expressions. It adds:

- `TStmt::ScalarLoanScopeEnd`: a checked lexical end marker emitted at
  the end of nested `block` or `if` arm scopes for any loan that has
  not already been NLL-retired. This emits **no Wasm guest bytes**.
- `check::loan_cfg_oracle_v1::audit_checked_program(&typed)`: builds a
  bounded basic-block-like point graph from actual checked typed
  assignments, returns, borrow issue/use/end markers and `if/else` arms.
- A backwards **MAY-future-use** fixed point derived from typed
  `LoanRead` instructions, not from `ScalarLoanEnd` or lexical close
  markers. At every claimed loan end, it proves no reachable successor
  has another typed use of the same loan.
- Forward active-loan checking for owner-write conflicts, exclusive
  owner reads, invalid loan-use/owner identity, missing closure and
  branch joins with mismatched active-loan states (which reject).
- One-to-one reconciliation of read/write event markers with the actual
  typed expression operands, separately from the liveness algorithm.

## Supported profile

Pure `int`, `bool`, `float` programs, bounded scalar locals,
straight-line statements, nested lexical blocks and `if/else` arms
(maximum four arms per conditional). Each branch must have the same
active loan state when it rejoins; branch-local loans can close at
checked lexical scope exit. The analysis uses closed limits:
1,024 point nodes, 256 loan IDs, 16,384 expression visits,
96 expression levels, 64 nested blocks, and 65,536 fixed-point steps.

Any of the following is **unsupported and rejected by the diagnostic**:
loops, break/continue, mismatched states at joins, checked arithmetic
traps or expression ternaries while loans are live, function calls,
unwind/exception/cancellation paths, generators, heap/class/actor data,
FFI, borrowed references escaping across domains, and any unsupported
typed effect. Existing language compilation still behaves as before.

## Proof boundary

The oracle is separately computed from **typed expressions and CFG**
instead of using the NLL `End` marker as a liveness oracle. However,
it remains in the same compiler and the source producer could itself
omit an emitted use. Emitted-Wasm operation witness checking, SSA phi
and checked arithmetic exceptional control-flow are **not** proven.
The independent `atto-borrow-checker.rs` PR #41 is not linked into
this compiler or compared on real CFG source facts in this stage.

The next PR should add an external checker adapter and emitted guest
operation receipts, then prove typed `LoanRead` coverage with exact
source+Wasm instruction accounting. Do not claim Polonius equivalence,
loop lifetime safety, or backend parity based on this restricted slice.

## Verification

Run pinned Rust 1.85.1 Rustfmt/Clippy, the native Rust test suite,
selected six positive / five negative scalar NLL guests, native/Wasm
byte parity, full conformance and Chromium E2E against a full
source-tree SHA-identical private Actions carrier when original
source runner minutes are unavailable. Never use cross-org checkout
tokens, and record actual checked out root tree and run status.

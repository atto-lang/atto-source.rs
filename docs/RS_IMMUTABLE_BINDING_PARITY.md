# Rust scalar immutable-binding parity slice (2026-11-07)

This source-local increment admits canonical immutable-by-default `int x = ...;`, `const int x = ...;`, `const x = ...;`, and scalar `val x = ...;` alongside the existing `const x: int = ...;`. A `val x: int` spelling is admitted as the same scalar local. Every admitted expression is already an `int` in the Rust prototype: this is **not** general type inference, ownership or borrow-checking parity. Explicit scalar `let`/`mut` bindings and `=`, `+=`, `-=` rebinding are separately implemented via capture-avoiding SSA-style versions on the same integrated branch.

The parser now ignores pseudo-block delimiters inside line comments when preprocessing nested block comments. This closes a source admission discrepancy without removing the existing nested-comment and unterminated-comment checks.

Evidence paths:
- `tests/immutable_binding_parity.rs`: AST equivalence, paired compiler/effect equivalence, duplicate and forward-name rejection, immutable-reassignment denial and unsupported-mutation denial, malformed numeric literals, comment nesting and spoofing.
- `tests/fixtures/immutable-bindings.atto`: an executable Rust scalar fixture (expected `main` exit status 42).
- `.github/workflows/ci.yml`: compiles, links and runs that fixture after `cargo test --locked --all-targets`.

**Not yet verified:** No hosted or local Cargo run, no same-source Java/LLVM receipt, and no physical execution parity is claimed by this PR. Keep draft and require successful exact-head steps for `cargo fmt --all -- --check`, `cargo test --locked --all-targets`, `cargo clippy --locked --all-targets -- -D warnings`, plus the native fixture and independent reviewer approval before merging.

Remaining gap: branch-aware mutation and canonical non-scalar bindings, nominal classes/constructors, borrow/move/share operators, control flow, actors, structured suspension, runtime effects, generation fencing, and ABI parity need separate compiler-backed implementations and cross-backend traces.

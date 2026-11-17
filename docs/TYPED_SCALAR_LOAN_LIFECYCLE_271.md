# Typed-MIR scalar loan lifecycle markers — issue #271, stage 2a

**Diagnostic-only / draft. NOT authoritative borrow admission or complete Polonius compatibility.**

This slice stacks on compiler PR #279. The actual type/ownership checker assigns each
scalar `borrow!` alias a unique, monotone `loan_id` in addition to its checked
owner slot and shared/exclusive kind. It keeps typed-IR markers:

- `ScalarBorrowIssue { loan_id, owner_slot, alias, exclusive }`
- `ScalarLoanUse { loan_id, owner_slot, write }`
- `ScalarLoanEnd { loan_id, owner_slot }` at the exact source checker NLL
  retirement boundary, before the following source statement.

They are ordered typed facts, never external data supplied by a caller. The
Wasm backend emits no bytes for them: scalar alias reads/writes continue using
the same previously checked local slot. `checked_borrow_issues` includes the
stable loan ID. The strict, opt-in `audit_scalar_loan_lifecycle` independently
cross-checks uniqueness, owner/exclusive kind, use-before-end, double ends,
and conflicting scalar local stores against a bounded, pure straight-line
typed-MIR statement stream. Borrowed returns, conditional CFG, loops, calls,
exceptions, actor effects, heap objects, and incomplete lexical cleanup
continue to **reject** rather than fabricate an alias/exception proof.

## Limits and nonclaims

**The trusted NLL source scanner still decides when a loan ends.** The new
audit checks event consistency but does not prove that every source alias
read is represented; it is NOT an independent liveness verifier yet. Markers
are currently associated with enclosing typed statement boundaries: precise
intra-expression evaluation ordering and early exceptional edges are not
represented. Therefore the audit accepts only a closed straight-line
pure-scalar subset with explicit end events and no escaping borrows.

A loan that remains alive up to a return cannot be certified by this
subprofile without explicit lexical scope exit and cleanup facts. Such safe
source programs continue to compile normally but the strict audit rejects.

## Next stage

Build an independently checked typed-to-emitted instruction/effect census;
precisely model alias reads and rights in subexpressions, lexical scope ends,
condition-specific SSA phi/CFG edges, loop fixed points, checked arithmetic
traps/unwind, `await`/cancellation, FFI, and actor domain boundaries. Then
differentially compare the canonical borrow checker without overriding any
existing production rejection. Compare actual native and Wasm executed guests
and preserve identical tree checks across private Actions carriers.

## Executed checks

```sh
rustup component add rustfmt clippy --toolchain 1.85.1
rustfmt --edition 2021 --check src/check/typed_mir_census.rs tests/typed_mir_scalar_census.rs tests/scalar_loan_lifecycle_mir.rs
cargo clippy --locked --lib --bin attoc -- -D warnings
cargo test --locked --verbose
cargo build --release --locked --bin attoc
cargo rustc --release --locked --lib --target wasm32-unknown-unknown --crate-type cdylib -- -C link-arg=--max-memory=67108864
node tests/scalar_nll_selected.mjs
node tests/conformance.mjs
node tests/ownership_wasm.mjs
node tests/browser-e2e.mjs
```

Run on exact source HEAD root-tree via token-free SHA-verified private Actions carrier
if source runner cannot allocate jobs. Never count skipped tests as success.

# Atto scalar loan-use / loan-end provenance (issue #271, stage 2)

**Status: draft, compiler-derived experimental diagnostic model.**
No production permission, new guest pointer, escaping reference, universal Polonius
parity or cross-backend safety certification is implied.

## Implementation

This PR stacks on atto-source.rs [#279](https://github.com/atto-lang/atto-source.rs/pull/279),
which stacks on scalar NLL [#273](https://github.com/atto-lang/atto-source.rs/pull/273).
The *real type/ownership checking pass* now issues monotonic per-function loan IDs
only after validating a named scalar owner and the shared/exclusive conflict rule.

Instead of erasing the loan as an empty typed block, it emits these **zero-Wasm**
typed markers alongside ordinary expressions and assignments:

- `ScalarBorrowIssue { loan_id, owner_slot, alias, exclusive }`
- `ScalarBorrowUse { loan_id, owner_slot, write }` at every admitted checked
  alias read or exclusive write-through.
- `ScalarBorrowEnd { loan_id, owner_slot, scope_exit }` at the previously
  proven NLL last-use boundary, or on lexical scope fallthrough.

`Return` terminates nonescaping scalar loans; the source checker rejects
returning or moving a loan alias itself. Scope-end events after an abrupt
return are unreachable, so the strict linear validator rejects such
control-flow rather than inventing an exception-aware CFG.

The pending use queue is **scoped** so condition uses cannot be smuggled into
an inner branch; borrow IDs are not recycled when aliases are shadowed.
Each alias write-through is recorded before the actual typed owner slot
`Set`. Other source owner writes are still independently checked by the
compiler's existing owner-loan restrictions.

## Side-by-side diagnostic API

`compile_with_scalar_loan_census(source)` runs parser, production type+borrow
checker, the new `check::scalar_loan_census::from_checked_program` on
the **same checked TProgram**, then the unmodified Wasm emitter. It reports
ordered one-based events, per-slot scalar SSA origins, and conservative
possible arithmetic-trap information. Its verdict is **diagnostic only** and
never used to weaken normal `compile` acceptance.

Its independent verifier checks issue-before-use, unique IDs, matching
owner slots, shared/exclusive conflicts, NLL end after a prior checked use,
no read/write after end, and write-through pairing with owner assignments.
Default `compile` never depends on an affirmative result from this model.

It rejects effects it cannot model accurately: `if`, `match`, loops,
ternary, callbacks/calls, heap/field/actor operations, suspension,
unwind/cancellation, unproved alias/FFI effects and non-scalar storage.
No caller-authored facts are accepted by the compiler entrypoint.

## Important verification gap

Even though the typed alias events are created from actual checked source,
they are not **independently authenticated for completeness against every
emitted Wasm local read/write**. The independent verifier checks their
consistency, but does not yet prove the type checker reported *all* alias
uses or every exceptional edge. Do not treat passing diagnostic checks
as a codegen authorization or as an external Polonius equivalence proof.

The canonical Rust checker at
[atto-borrow-checker.rs PR #41](https://github.com/ores-truffle-oreslang/atto-borrow-checker.rs/pull/41)
remains a separate *experimental host model*. Linking its typed-MIR facts,
verifying correspondence with actual emitted operations and differential
proving stronger branch-/loop-sensitive alias invariants are next stages.

## Testing

New `tests/scalar_loan_lifecycle.rs` checks compiler-generated events on
real AttoLang positive fixtures, the same byte-identical guest Wasm as
`compile`, writer permission, shared-to-exclusive transition, lexical
scope endings, return closure, deterministic emission, and negative
mutations of checked typed IR. Existing native-vs-Wasm binary/guest
conformance, browser worker E2E and six positive/five negative scalar NLL
fixtures run in CI.

Source/actions minutes limitation: mirror the **complete exact Git tree**
to an approved private carrier by GitHub blob SHA; use the carrier's own
local checkout without credentials crossing organizations.

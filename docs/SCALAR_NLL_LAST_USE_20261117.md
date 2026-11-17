# Restricted compiler-local scalar non-lexical lifetime proof (2026-11-17)

This slice implements **real executable AttoLang source-level NLL**, in
`src/check.rs` + `src/check/scalar_lifetime.rs`, stacked on compiler
scalar-loan draft PR #141. It is neither Rust Polonius Alpha nor the
`atto-borrow-checker.rs` typed-MIR oracle.

A borrowed **scalar** alias in its declaring lexical scope may be retired at
the start of a **later** statement only if:

* It had already been actually used by the normal typed expression checker
  (or a verified exclusive write-through);
* Every remaining statement in this same lexical body has a recognized
  straight-line shape and its expressions have been traversed completely;
* None of those remaining statements references the alias; expression depth,
  statement count and active aliases stay within bounded scanner limits;
* The code is not inside a loop. An outer-scope alias is not shortened by
  reaching the end of a nested branch or block.

All other cases retain the existing lexical loan. The alias retains its
borrowed identity even after release, so any unexpected subsequent access
raises `E_BORROW` instead of being accidentally treated as an owned value.
No guest pointer is manufactured. The same compiler pass checks and lowers
the owner's scalar slot. No user-entered lifetime facts are trusted.

### Concrete source example

```atto
pub fnc main(): int {
  let number = 40;
  val reader = borrow! number;
  val snapshot = copy! reader;
  number = 42; // now permitted: reader's last use was earlier
  return number;
}
```

An exclusive borrowed writer can similarly be used and retired before a
subsequent owner assignment. An exclusive writer can also be taken after a
shared alias's final checked use. Actual compiler/guest test fixtures:
`tests/fixtures/pass/ownership-borrow-nll-*.atto` and
`tests/fixtures/fail/ownership-borrow-nll-*.atto`.

### Nonclaims and integration next

The host-only canonical oracle's `mir_lifetime_census_v1` manifests are
NOT yet emitted by this compiler and are NOT approved as a codegen gate.
This scoped implementation uses the browser compiler's own AST and
non-escaping typed loan metadata; the canonical Rust oracle is an
independent differential test objective, not a replacement for these checks.

No container/object reference escapes or returned borrows, Polonius-style
CFG/region inference, SSA dominance/phi, NLL inside loops, heap aliasing,
actor handoff, pinned `await`/`select`, FFI, throw/unwind or cleanup
proof are implemented. Unknown constructs keep the prior rejection behavior.
The deliberately narrow use-before-retirement rule also leaves unused
borrowed aliases lexical (legacy compatibility; conservative).

A follow-on must introduce a fully typed source/MIR effect census with
independent coverage and emitted-code provenance, then differential
compare the shared canonical engine to compiler-local decisions before
generalizing to `Option` returned references or other backends.

### Test gate

Run native Rust tests, the real native and wasm32 AttoLang compiler builds,
`node tests/conformance.mjs` against executable guests with native/Wasm
byte comparison, `node tests/ownership_wasm.mjs` for zero-artifact
rejections, and browser worker E2E. If source CI cannot execute, use
the same SHA-verified exact-source-tree private GitHub Actions carrier
as PR #141. Keep this work draft until executed evidence exists.

## Hardening audit (2026-11-17)

Audited the AST-based retirement algorithm for two under-specified proof
boundaries and corrected both without broadening the admitted ownership
profile:

1. **Unproved effectful tail:** calls, method calls, object creation,
   spawns, member/heap references, user-defined callbacks, trap, early
   propagation, container expressions, unproved reborrows and take operations
   were previously inspected as though ordinary pure expressions could
   prove a suffix harmless. The scanner now returns "unproved" for these
   expression categories, so their presence keeps the loan live until
   lexical scope exit. Only fully checked scalar literals, named scalar
   references, admitted pure unary operations, arithmetic/boolean and
   ternary expressions can justify retirement, as can a fresh borrow of a
   **named scalar owner** when the typed borrow gate proves its mutability
   and conflicts. A new borrow of the old alias remains a live use.
   This deliberately causes
   some conservative E_BORROW rejections for safe but not yet proved
   effectful programs; it does not authorize guest code with unverified
   runtime/cleanup effects.
2. **Aggregate analysis budget:** the earlier 4096-node limit was
   applied to each loan/suffix scan, allowing repeated scans to consume
   excessive compiler time. In addition to the 256-statement/single-scan
   bounds, the compiler now tracks an **entire-function shared 32768
   scan-work-unit budget**; exhaustion retains remaining loans (fail
   closed). Writer aliases also have a direct retired-state check at
   assignment as defense in depth, independent of the scanner's census.

Additional Rust unit tests exercise all rejected effectful expression
families, pure nested expressions, scanner work exhaustion and statement
limits. Additional real AttoLang guest fixtures test a pure suffix as a
positive case and an unproved call or ownership-transfer suffix as
conservative negative cases.

These restrictions are not a substitute for future independent typed-MIR
effect inventories. They specifically avoid guessing purity or control
flow from the surface AST until those compiler proofs exist.

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

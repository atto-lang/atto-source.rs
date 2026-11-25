# Implicit argument borrowing: compiler migration and safety gates

**Draft compiler workstream. Not implemented as a general language default.**
Canonical target: [Atto spec #89](https://github.com/atto-lang/atto-spec/pull/89), [language chapter #80](https://github.com/ores-truffle-oreslang/attolang-language-spec.md/pull/80), [conformance #74](https://github.com/atto-lang/atto-platform-conformance/pull/74).

## Current bounded compiler capability

The stacked Rust compiler [PR #332](https://github.com/atto-lang/atto-source.rs/pull/332) accepts synchronous, named local class arguments to explicit **readonly `Borrow<Foo>`** top-level function formals; those ephemeral loans are checked across later arguments and the caller retains ownership. This branch adds the logically equivalent **explicit `borrow! foo` argument** for the very same read-only formal. Both code paths select the same local owner and issue the same checker-only `ScopedLoan` with no additional class-owned local or heap copy. Mixed `Borrow<Foo>` and owned `Foo` signatures remain distinct in the existing ABI.

Examples covered by this narrow change:

```atto
fnc read(p: Borrow<Person>): int { return p.age; }
let person = new Person();
read(person);          // existing #332: implicit read loan
read(borrow! person);  // this PR: equivalent, explicit spelling
```

**Not yet supported:** the requested default `fnc read(Person p)` -> borrow, `read(mut p)` for mutable loans, callee-effect inference, general `Borrow<T>` lifetimes, methods/constructors/actors, async/await, argument projections and proven return-borrow ABI. Current historical `fnc read(Person p)` still *moves* the class value, including some existing tests; do not call this the canonical rule in production.

## Soundness checkpoints before defaulting bare nonprimitives

1. **Breaking migration and owned-formal grammar.** Existing `Foo` formals consume by default. Switching them to read loans without a versioned module boundary breaks formerly consuming APIs and can incorrectly retain an owner; require an explicit consuming-formal contract and `take!` call site.
2. **Mutability inference.** Statically summarize transitive writes from each function/method body and seal mutable parameter requirements before signature export/call admission. Unknown, recursive-unresolved, indirect, native, host and dynamic effects must fail closed. `mut` call qualifier must not grant referent-write permission that the caller lacks.
3. **Typed provenance.** Preserve non-owning formals as actual loan types in checked MIR, with one origin and lifetime tracked across calls, cleanup, throws, `await`, CFG joins and allocation domains. Existing lowered formal class-handle slots are **not** proof of drop/escape safety for general borrowed objects.
4. **Escapes and actors.** A returned/captured/stored borrowed reference may not outlive its owner. Actor-private memory never crosses a mailbox/isolate boundary merely by omitting or adding `borrow!`.
5. **Argument order.** Resolve named arguments first, then issue loans in source-expression order and hold them through later argument effects; no hidden extra evaluation or handoff.
6. **Backend proof.** For every backend require exact source SHA/tree, executable compiled positive guest outputs, deterministic negatives with *zero generated artifact*, lifetime-conflict tests and runner receipts with nonempty job steps.

## Authored regression fixtures

- `194-auto-borrow-explicit-call-spelling.atto`: explicit and inferred readers preserve ownership and produce 42.
- `195-auto-borrow-two-explicit-reader-args.atto`: two compatible readers in one call produce 42.
- `208-auto-borrow-explicit-mut-not-readonly.atto`: reject incorrectly requesting an exclusive loan through read-only formal.
- `209-auto-borrow-explicit-temporary-not-owner.atto`: reject explicit borrow on an unproved temporary.

**Evidence:** These tests were authored on the compiler branch. No Rust tests, native build, actual Wasm guest or carrier CI is claimed executed for this new commit. Keep the PR DRAFT and stacked until validated.

# Opt-in borrow-v1 compiler implementation

**Status: bounded executable read-only migration mode (draft).**

Current default `compile(source)`, wasm `compile(length)` and native `attoc file.atto` retain the **historical owned class formal/move ABI**. That old behavior must not change for already existing programs until the user-approved migration of explicit consuming formals is complete.

To exercise the target plain nonprimitive-parameter model in the Rust/browser subset:

```text
attoc --borrow-v1 input.atto -o guest.wasm
```

Rust callers can use `compile_with_mode(source, OwnershipMode::BorrowV1)`. The standalone browser compiler exports `compile_borrow_v1(length)`, using the same input/output/error pointer ABI as `compile(length)`; existing clients see no change.

In this versioned mode a top-level synchronous `fnc inspect(Person p)` plain class formal is internally normalized to the **same verified `Borrow<Person>`** typed signature as its explicit long-form spelling. It is a *read-only, nonescaping* loan with permanent borrowed provenance even after NLL loan retirement. The original owner remains in the caller; `inspect(person)` and `inspect(borrow! person)` perform the same guest instructions, not clone, `share!`, refcount or take. Primitive `int`/`bool`/`float`/enum arguments and owned results are unchanged.

### Explicit consuming formals (opt-in only)

The versioned profile now admits `fnc transfer(take! Person owner)` and
`fnc transfer(owner: take! Person)`. These bind an **owned** class parameter,
in contrast to a plain `Person` read-only formal. Their caller must explicitly
write `transfer(take! owner)` where `owner` is a live named owning local.
The ordinary `transfer(owner)` form fails with `E_OWNERSHIP`; a second
transfer or later owner use fails with `E_MOVED`. Passing `borrow! owner`,
a temporary, or an alias is not accepted as a transfer. This is a compiler
ownership effect, **not a new runtime ABI type**.

Existing unversioned `compile(source)` and `attoc input.atto` reject
declarations using consuming-formal syntax rather than silently mapping them
to legacy implicit moves. Mutable, scalar, method, constructor and actor-domain
consuming formals remain outside this bounded profile.

### Fail-closed boundaries

- `fnc steal(Person p): Person { return p; }` rejects: a borrowed formal cannot be returned as owned.
- `fnc change(Person p): void { p.age = 42; }` rejects: a borrowed formal cannot mutate without a verified exclusive-parameter and `mut` call contract.
- Passing computed `make()`, `take! p`, a borrowed alias with unverified reborrow lifetime, conflicting loans, and unsafe owner escape rejects.
- Class method/constructor and actor method *class-typed formals* reject in this mode; these require a separate receiver lifetime and class/actor ABI review. **Do not silently use the old moving semantics for those formals in borrow-v1.**
- `Borrow<T>` return types, `BorrowMut<T>`, named `lt! a` lifetime relations, FFI, await/select, generics and cross-actor borrows **remain unimplemented** with no emitted artifact. A source-level `T` return is still **owned**, not auto-borrowed.

### Safety and test evidence

Run `cargo test --locked --test borrow_v1_mode` and compile/execute the dedicated `tests/fixtures/borrow-v1` matrix using both native and browser compiler **opt-in** entry points. Require positive guest execution, diagnostics plus zero emitted guest bytes for negatives, exact-source Git blob verification, SHA pinning, and byte parity.

The implementing PR must remain draft until these checks run and the broader [migration issue](https://github.com/atto-lang/atto-source.rs/issues/338), [spec #89](https://github.com/atto-lang/atto-spec/pull/89) and [lifetime #78](https://github.com/ores-truffle-oreslang/attolang-language-spec.md/pull/78) are reviewed.

**This is not general automatic borrowing certification.** The default migration awaits explicit consuming formal grammar, transitive mutable effects, return-lifetime provenance and backend parity.

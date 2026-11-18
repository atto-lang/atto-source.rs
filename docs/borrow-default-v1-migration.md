# Versioned bare-class borrowing migration (experimental)

**Status: DRAFT compiler profile, not final package syntax or five-backend ratification.** This branch builds on the stacked source compiler loan/checked-handle work and the already approved conceptual rule that ordinary `Foo` parameters borrow by default. It preserves every legacy source file's original ABI unless it **explicitly** opts in.

## Enable the v1 migration in a single source file

The **first source line** must be exactly:
```atto
// atto-ownership: borrow-default-v1
```

This is a temporary compiler migration header until project-level configuration and the formatter's versioned ownership contract are ratified. It does not create a new language keyword. An unknown header at the first line rejects with `E_PROFILE`; no run-time detection or silent fallback. Files without this header compile using the existing legacy consuming-`Foo` ABI.

## Ownership contract in this narrow compiler slice

```atto
// atto-ownership: borrow-default-v1
define class Parcel as pub let int value = 42; end

fnc inspect(Parcel p): int { return p.value; }
fnc consume(take! Parcel p): int { return p.value; }

pub fnc main(): int {
  let parcel = new Parcel();
  val a = inspect(parcel);           // implicit readonly loan
  val b = inspect(borrow! parcel);   // identical explicit readonly loan
  return a + b;                      // owner remains live
}
```

- Bare **class** formals in **top-level** `fnc` use the *same existing* typed `Borrow<Class>` checker and non-owning Wasm handle ABI. This is not a clone, refcount, share, or move.
- The **existing** `take!` keyword now marks a consuming formal, as `fnc consume(take! Parcel p)`; the caller must use `consume(take! parcel)`. The old owner is invalid afterwards.
- Scalar, enum and ActorRef formals remain unchanged. Explicit `Borrow<T>` still works; no hidden mutable loans.
- Bare `Foo mut p` in this profile rejects. `update(mut foo)` needs the separate, future `BorrowMut` and CFG lifetime proof.
- Class constructors/methods and actor methods with bare class formals are explicitly rejected in the v1 profile until receiver/arena effects and sealed actor provenance are fully verified; don't silently use the legacy owned behavior there.
- Caller/callee loans retain existing negative behavior: no read-to-write, no owned return from borrowed formal, no owned call from borrowed formal, no moving a live borrowed owner, no class borrow of temporary.
- Legacy source remains unchanged for compatibility, even though the final AttoLang language design intends borrow-by-default everywhere. This is an incremental, reviewable migration stage, **not** completion.

## Verification

The source fixture corpus includes 4 expected executable guests and 9 rejected invalid programs. The native fixture harness and full native-vs-browser Wasm conformance harness pick them up automatically. To claim implementation, require exact source/carrier blob identity, positive real Wasm guest exit/stdout, negative expected diagnostics with **zero bytes emitted**, and regression tests proving the no-header legacy ABI remains owned.

The header is intentionally experimental and must be reviewed with [atto-spec #89](https://github.com/atto-lang/atto-spec/pull/89), [compiler #336](https://github.com/atto-lang/atto-source.rs/pull/336) and [PR #345](https://github.com/atto-lang/atto-source.rs/pull/345) before language-spec/formatter-wide adoption.

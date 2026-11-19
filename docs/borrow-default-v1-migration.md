# Versioned bare-class borrowing migration (experimental)

**Status: DRAFT compiler profile, not final package syntax or five-backend ratification.** This branch builds on the stacked source compiler loan/checked-handle work and the already approved conceptual rule that ordinary `Foo` parameters borrow by default. It preserves every legacy source file's original ABI unless it **explicitly** opts in.

## Enable the v1 migration in a single source file

The **first source line** must be exactly:
```atto
// atto-ownership: borrow-default-v1
```

This is a temporary compiler migration header until project-level configuration and the formatter's versioned ownership contract are ratified. It does not create a new language keyword. An unknown header at the first line rejects with `E_PROFILE`; no run-time detection or silent fallback. Files without this header compile using the existing legacy consuming-`Foo` ABI. A misplaced header on a later line rejects with `E_PROFILE` rather than silently reverting to legacy ownership.

## External profile selection (native build drivers)

The first-line source marker is retained as a backward-compatible temporary migration bridge. The native `attoc` compiler now also supports explicit profile selection **without** editing source:

```bash
attoc input.atto --ownership-profile borrow-default-v1 -o app.wasm
attoc input.atto --ownership-profile legacy -o legacy.wasm
```

An embedding compiler may instead call `compile_with_profile(source, OwnershipProfile::BorrowDefaultV1)`. If an explicit profile conflicts with the source header, compilation fails with `E_PROFILE` and emits no guest module. Unknown and duplicate CLI flags reject, and a missing option retains the historical source-marker/default behavior.

**No automatic manifest discovery is implemented yet.** The build driver must select its package's ownership version explicitly. The eventual package TOML field, cross-module export/import ownership metadata and formatter integration require separate spec review: a caller must never silently treat an imported owned formal as a borrow, or vice versa.

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
- This stacked #347 compiler additionally admits `fnc update(mut Foo p)` with `update(mut owner)` or `update(borrow! mut owner)` for synchronous, exclusive top-level class loans under the explicit profile; it rejects conflicting reads/writes and retains unsupported async, actor, method and constructor limits. The #346 parent alone does not implement mutable loans.
- Class constructors/methods and actor methods with bare class formals are explicitly rejected in the v1 profile until receiver/arena effects and sealed actor provenance are fully verified; don't silently use the legacy owned behavior there.
- Caller/callee loans retain existing negative behavior: no read-to-write, no owned return from borrowed formal, no owned call from borrowed formal, no moving a live borrowed owner, no class borrow of temporary.
- Legacy source remains unchanged for compatibility, even though the final AttoLang language design intends borrow-by-default everywhere. This is an incremental, reviewable migration stage, **not** completion.

## Operator status in this opt-in profile

| Operation | This compiler branch's admission boundary |
| --- | --- |
| `inspect(owner)` / `inspect(borrow! owner)` | Same scoped readonly class loan under the first-line opt-in only |
| `consume(take! owner)` | Requires a matching consuming `take! Class` formal; caller loses ownership |
| `update(mut owner)` / `update(borrow! mut owner)` | **Admitted by this stacked #347 compiler only** for exclusive synchronous top-level class-loan formals under `borrow-default-v1`; not a general mutable loan |
| `share! owner` | No automatic frozen sharing from a default borrow; separate capability verification required |
| `copy! owner` / `copy!! owner` | No implicit copying at call sites; copy legality and graph plan are separate features |
| `pin! take! owner`, `drop! owner;` | No implicit pinning or release during a borrowed call; independent backend/destructor checks required |
| `freeze! take! owner`, `weak! shared` | Not granted by the narrow default-borrow migration |

The checker must reject a call that tries to consume or retain a borrowed
formal, including through a return value, cloned handle, actor send or
suspension. Explicit `borrow!` cannot relax any of these rejections. The
versioned migration profile does not ratify package-wide ownership defaults.

## Verification

The source fixture corpus includes 4 expected executable guests and 11 rejected invalid programs. The native fixture harness and full native-vs-browser Wasm conformance harness pick them up automatically. To claim implementation, require exact source/carrier blob identity, positive real Wasm guest exit/stdout, negative expected diagnostics with **zero bytes emitted**, and regression tests proving the no-header legacy ABI remains owned.

The header is intentionally experimental and must be reviewed with [atto-spec #89](https://github.com/atto-lang/atto-spec/pull/89), [compiler #336](https://github.com/atto-lang/atto-source.rs/pull/336) and [PR #345](https://github.com/atto-lang/atto-source.rs/pull/345) before language-spec/formatter-wide adoption.

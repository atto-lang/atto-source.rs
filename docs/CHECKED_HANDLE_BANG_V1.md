# Checked-handle unary operators — bounded Rust/Wasm profile

**Status: DRAFT / compiled guest NOT YET VERIFIED.** See [PR #264](https://github.com/atto-lang/atto-source.rs/pull/264), [syntax spec PR #67](https://github.com/ores-truffle-oreslang/attolang-language-spec.md/pull/67), and [conformance issue #63](https://github.com/atto-lang/atto-platform-conformance/issues/63).

The compiler accepts the following canonical syntaxes and the corresponding `rt` compatibility forms in the same typed AST:

| Canonical | Legacy | Transfer |
| --- | --- | --- |
| `ref! owner` | `rt ref owner` | Consume the unique class owner into a `CheckedRef<Class>` token |
| `deref! handle` | `rt deref handle` | Create a scoped, immutable class view with the handle as loan owner |
| `unref! handle` | `rt unref handle` | Consume the handle exactly once and return the owned class |

## Executable subset

Only **single-worker, top-level function, bump-allocated class values** are admitted. This profile's reference is a WebAssembly `i32` allocation offset in a monotonically allocated guest heap. It is not a host address. The compiler alone mints the `CheckedRef<Class>` type when it consumes a unique class-local owner, so source code cannot supply numeric handles. Because this guest heap does not free or reuse class allocations during the function lifecycle, the profile relies on lack of allocation reuse rather than claiming generic runtime generation/ABA checks.

```atto
define class Parcel as
  pub let int value = 42;
end
pub fnc main(): int {
  val original = new Parcel();
  val handle = ref! original;
  block {
    val view = deref! handle;
    stdio.println(view.value);
  }
  val recovered = unref! handle;
  return recovered.value;
}
```

### Fail-closed restrictions

- Only a **named owned class local** is accepted by `ref!`; arbitrary computed expressions, scalars, actors and foreign memory do not become checked handles.
- `deref!` must directly initialize an immutable (`val`) scoped view; it cannot grant mutable access, escape as an owned object, or reborrow an existing view without proven provenance.
- `unref!` must directly initialize a new owned binding, must not run with any live view, and cannot be nested in ternaries or other expressions.
- Every locally minted handle must be consumed; plain aliasing, copying, sharing, use-after-`unref!`, double-`unref!` and missing `unref!` fail compilation.
- **No branching, loops, returns or exception paths with live handles** in this first profile; those paths require a verified typed-CFG cleanup proof. Ordinary nested `block` scopes can contain views, which must retire lexically before `unref!`.
- Code inside actors and class methods, cross-worker transport, FFI, raw pointers, mutable dereferencing, explicit allocation reuse, and detached suspension are not supported.
- Failure of any guard produces a diagnostic and **no guest Wasm artifact**. The browser compiler does not claim to replace the master Rust borrow-checker for full programs.

### Verification requirements

`cargo test --locked` must compile the positive and negative sources; `node tests/conformance.mjs` must use the actual wasm32 **compiler** to produce and execute guest Wasm, compare its output with the native compiler bytes, and assert no output artifact for failures. The canonical and `rt` forms must produce equivalent guest behavior. The current source-org workflow could not allocate an execution runner (zero steps); no compiled guest success is claimed here.

**Next stage:** integrate a typed handle creation/access/retirement inventory into `atto-borrow-checker.rs` and give all independent backends authenticated actor/arena/allocation identities, generation/ABA fencing, allocation extent/bounds/alignment, explicit physical reclamation and runtime validation before expanding this profile. Never silently substitute raw addresses or ordinary borrow semantics.

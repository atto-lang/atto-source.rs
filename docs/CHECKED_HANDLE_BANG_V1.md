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

### Canonical scoped block admission

Use **`block { ... }`** to introduce an independent lexical loan lifetime inside a callable. Bare `{ ... }` at statement position is **not** interchangeable syntax; the parser rejects it rather than silently manufacturing a scope. Braces used for callable bodies, `if` arms, class declarations and other constructs keep their existing grammar and are not affected.

```atto
val original = make_payload();
val handle = ref! original;

block {
  val view = deref! handle;
  stdio.println(view.value);
}

val restored = unref! handle;
```

The `block` statement must retire all view loans when its lexical scope exits. Multiple read-only views are admissible within the same `block`, but `unref!` before its closing brace fails. A view cannot escape the lexical scope or acquire mutable authority. The current first slice permits the `view.field` and `view.method()` forms proved by its type checker; a read-only view can be passed only to a function that explicitly declares a `borrow! Class` parameter; ordinary by-value function arguments still transfer ownership.

### Explicit borrowed function parameters — first executable proposal

A function must **declare** that it accepts a non-owning class loan using `borrow! Payload name`. A caller then writes the ordinary function invocation `inspect_readonly(view)` exactly as in the canonical user-facing example. This is **not** a change to the usual move semantics of `fnc consume(Payload name)`: the latter still consumes an owned argument and rejects a scoped view.

```atto
define class Payload as
  pub let int value = 42;
end

fnc make_payload(): Payload { return new Payload(); }
fnc inspect_readonly(borrow! Payload item): int {
  return item.value;
}

pub fnc main(): int {
  val original = make_payload();
  val handle = ref! original;
  block {
    val view = deref! handle;
    stdio.println(inspect_readonly(view));
  }
  val restored = unref! handle;
  return restored.value;
}
```

The compiler resolves `borrow! Payload` as a distinct signature type, lowers both the signature and the call argument to the ordinary class-reference `i32` Wasm ABI, and treats the callee's parameter as a readonly lexical loan. Callees cannot move or return the borrowed class, assign its fields, or forward it to an owned argument. They may forward the loan to another explicitly `borrow!`-typed synchronous function. Only *named class bindings* are admitted as borrowed arguments, preventing temporary value lifetime ambiguities.

This is a deliberately restricted compiler profile: borrowed parameters are allowed only on top-level synchronous `fnc` declarations, not constructors, class/actor methods, generators, FFI, suspension, escaped closures or cross-worker sends. These are **authored tests and typed-source changes, not a completed real-Wasm verification**.

### Scoped, loan-preserving `do if` (Rust/browser profile)

The **canonical procedural spelling** for these conditional arms is `do if ... then ... else ... fi`. The parser lowers `do if` to the same typed conditional as the legacy `if`. This restricted form is now admitted while a checked handle is live **only if every branch preserves the outstanding handle and borrowed views**. `ref!` and `unref!` still require direct, top-level, straight-line `val` bindings; early return, mutable/escaping loans, loops, suspension, and exceptional exits remain rejected.

```atto
fnc inspect_readonly(borrow! Payload item): int { return item.value; }
fnc inspect_other(borrow! Payload item): int { return item.value; }

fnc inspect_branch(bool condition): Payload {
  val original = make_payload();
  val handle = ref! original;

  block {
    val view = deref! handle;

    do if condition then
      inspect_readonly(view);
    else
      inspect_other(view);
    fi
  }

  val restored = unref! handle;
  return restored;
}
```

In addition to local type checking, the compiler copies the pinned standalone Rust all-path CFG oracle from `atto-borrow-checker.rs` and lowers *type-admitted checked-handle source events* to `Ref`, `Deref`, `EndView`, `Unref`, normal/early `Exit`, and actual branch edges. It keeps distinct facts across a join, so cleanup on only one path cannot certify another. This is a deliberately bounded **typed-admitted AST-to-CFG preflight**, not yet a full typed-MIR proof contract or an exception-safe runtime destructor. Other backends must fail closed until equivalent enforcement is verified.

### Transitive guest unwind while a handle is live

A function call inside a `block` that still has an outstanding `ref!` handle is admitted only when the typed guest-call graph does not contain a known `throw`, `raise`, `trap`, `?` early-exit, actor send/spawn, or other modeled guest unwind path. The compiler computes a fixed-point reachability summary over typed direct calls, including instance calls. This rejects a wrapper that calls another function which throws, even if the outer wrapper's body contains no explicit `throw`:

```atto
fnc fail(): int { throw Error("boom"); }
fnc wrapper(): int { return fail(); }

// Rejected: wrapper() can bypass the later unref!.
val original = make_payload();
val handle = ref! original;
block {
  val view = deref! handle;
  stdio.println(view.value + wrapper());
}
val restored = unref! handle;
```

This is intentionally **not** a general exception-safe automatic destructor: no typed cleanup/unwind edges, cancellation cleanup or guarantees after a fatal Wasm trap or host termination are implemented here. Before permitting exceptions, suspendable tasks, or arbitrary control-flow regions containing checked handles, the compiler must connect a path-sensitive lifecycle proof to typed CFG/MIR and a real runtime cleanup mechanism. The separate `atto-borrow-checker.rs` CFG oracle is a first independent proof foundation, not yet wired to this compiler.

### Fail-closed restrictions

- Only a **named owned class local** is accepted by `ref!`; arbitrary computed expressions, scalars, actors and foreign memory do not become checked handles. In this initial profile `ref!`, `deref!` and `unref!` must each directly initialize a `val` binding. `const` is forbidden because these are runtime operations; `let` is deferred until a full rebinding/alias proof exists.
- `deref!` must directly initialize an immutable (`val`) scoped view; it cannot grant mutable access, escape as an owned object, or reborrow an existing view without proven provenance.
- `unref!` must directly initialize a new owned binding, must not run with any live view, and cannot be nested in ternaries or other expressions.
- Every locally minted handle must be consumed; plain aliasing, copying, sharing, use-after-`unref!`, double-`unref!` and missing `unref!` fail compilation.
- **No branching, loops, returns or exception paths with live handles** in this first profile; those paths require a verified typed-CFG cleanup proof. Ordinary nested `block` scopes can contain views, which must retire lexically before `unref!`.
- Code inside actors and class methods, cross-worker transport, FFI, raw pointers, mutable dereferencing, explicit allocation reuse, and detached suspension are not supported.
- Failure of any guard produces a diagnostic and **no guest Wasm artifact**. The browser compiler does not claim to replace the master Rust borrow-checker for full programs.

### Verification requirements

`cargo test --locked` must compile the positive and negative sources; `node tests/conformance.mjs` must use the actual wasm32 **compiler** to produce and execute guest Wasm, compare its output with the native compiler bytes, and assert no output artifact for failures. The canonical and `rt` forms must produce equivalent guest behavior. The current source-org workflow could not allocate an execution runner (zero steps); no compiled guest success is claimed here.

**Next stage:** integrate a typed handle creation/access/retirement inventory into `atto-borrow-checker.rs` and give all independent backends authenticated actor/arena/allocation identities, generation/ABA fencing, allocation extent/bounds/alignment, explicit physical reclamation and runtime validation before expanding this profile. Never silently substitute raw addresses or ordinary borrow semantics.

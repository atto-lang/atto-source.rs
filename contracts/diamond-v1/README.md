# Cross-backend diamond inheritance and dispatch contract v1

This folder contains a **byte-identical copy of the canonical @Override playground example**
proposed in [atto-lang.github.io PR #149](https://github.com/atto-lang/atto-lang.github.io/pull/149)
(the previous `pub override` preview landed in site PR #130):

- `playground-exact.atto`: Git blob SHA-1
  `91cdb52d2b099c1fe1892d9960866be5d011a076`.
- Identical source Git blobs must be checked across the Rust/Wasm, Java and LLVM
  branches. **Source agreement is not evidence of executable backend parity.**

## Required eventual semantics

1. A class may name more than one parent. Identical declarations reached
   through two diamond paths are **one origin**, not two conflicting members.
2. Unrelated implementations of the same **name + source-visible arity + member
   kind** are ambiguous. No implicit left-to-right, C3 or hash-order winner.
   The child must provide its own explicit resolution; the canonical playground
   spelling is `@Override` on the preceding line, followed by `pub describe(): string`.
3. Different explicit arities may coexist. Argument types alone do not create
   additional same-arity overloads. Override signatures and public visibility
   must be preserved; invalid attempts reject before emitting artifacts.
4. `Class.static_name()` is a nonvirtual **class-qualified static** lookup,
   separate from instance receiver slots. A base-typed instance method call
   on a child object must dispatch to the correct child override, not a
   frozen/base implementation or a guessed devirtualization target.
5. Interfaces impose signatures; traits may provide defaults. Distinct trait
   concrete defaults for one slot are ambiguous absent explicit resolution.
6. Inherited field layout, constructor initialization, receiver ownership,
   method visibility and eventual dynamic vtable/witness ABI must be verified
   before enabling unsupported shapes. An unimplemented backend must
   reject **without generated guest Wasm, LLVM IR, or a native binary**.

## Migration and current verification status (2026-11-16)

The branch parser now maps source-level `@Override` onto the same existing
`override_member` AST flag used by the already-tested legacy modifier. The
legacy `pub override` spelling remains accepted for compatibility; writing
both or repeating `@Override` is rejected. `@Override` on actor members,
static methods, fields, top-level class/actor/fnc/routine or otherwise
non-overriding methods is rejected. Unregistered annotations are still
fail-closed: this does not establish general annotation compiler support.

**The previous executed 2026-11-15 CI evidence below applies to the older
`pub override` fixture SHA only.** The new `@Override` source has not
yet passed a fresh exact-tree Rust/Wasm guest run. It must not be marked
executable or released until that evidence exists.

## Current evidence and non-parity boundaries (2026-11-15)

| Backend | Executed evidence | Missing parity |
| --- | --- | --- |
| Atto Rust compiler & its wasm32 build | [Private full-tree Rust 1.85.1 Actions run 43756999140](https://github.com/zed-pkg-test/oreslang-private-pr-validation-20261012/actions/runs/43756999140): 391 guest cases, 354 byte-parity comparisons, native Rust tests, Chromium guest/browser checks and exact example all passed | No general base-typed polymorphism, upcasts, virtual table, private base field inheritance or chained base constructors; cannot claim Java-style virtual dispatch |
| `oreslang-source.java` / JVM | Existing parser and `TypeChecker` model nominal multiple bases, name/arity slots, inherited-identity dedup and explicit child declarations; added JVM test cases in draft parity branch | Exact playground `override` keyword and concrete `trait` default bodies are not yet admitted equivalently; exact-source guest execution and Graal Native Image not proven |
| `oreslang-source.llvm` | Existing native compiler supports standalone concrete class instance methods and checked direct LLVM calls; this branch adds explicit fail-closed header diagnostics and negative codegen checks | Native parent-class layout, inheritance, trait default implementation, qualified static methods and virtual override codegen are **not implemented** |
| `atto-source.wasm` project | Does not implement a second compiler; uses artifact-driven tests for the Rust-authored compiler built as Wasm | Public compiler release/digest pin and live browser adoption still pending |

### Next release gates

- Java: implement and execute **exact** playground source rather than accepting an
  adapted source dialect; preserve checked dynamic base/child dispatch and
  class static-lookup semantics.
- LLVM: implement a typed parent layout and object identity/vtable/witness
  mechanism in actual C++ compiler/IR; execute the unchanged source and all
  negative cases at Clang O0/O2 with ASan/UBSan, not a host-only mock.
- Rust/Wasm: add base-typed runtime virtual calls and matching ownership/type
  rules, or keep them explicitly unsupported, with negative no-artifact tests.
- All: verify identical fixture blob SHA, exact commit/tree, compiler/toolchain,
  executed tests, stdout and guest artifact digests. No zero-step CI status or
  host-only source parse is a substitute.
- Playground: keep preview disabled until reviewed compiler binary disclosure,
  digest pinning, and deployed Chromium worker E2E.

**An explicit incomplete matrix is required; never mark these backends at full
inheritance parity merely because they accept the same source file.**

# Rust/Wasm backend-local core memory and atomic policy candidates

The Rust/Wasm compiler owns `stdlib/core/policy/mem.atto`, `ptr.atto`,
and `atomic.atto` independently. No compiler may silently substitute host
Rust implementations for source-owned Atto helpers.

`core/mem` checks alignment/stride/array spans, `core/ptr` checks
region-relative indices and alignment, and `core/atomic` validates memory
orders (0 relaxed, 1 acquire, 2 release, 3 acq_rel, 4 seq_cst). Invalid
integer results return -1; invalid predicates return false.

**These are scalar source-policy helpers only.** The existing Rust compiler
[PR #419](https://github.com/atto-lang/atto-source.rs/pull/419) separately
implements a bounded lexical-local `Pointer<int>` candidate, and `main`
intentionally fails closed for unlowered atomic types. This work must not
relax those gates. It cannot dereference memory, establish provenance,
authorize shared actor memory, implement atomic loads/stores or CAS/RMW, or
release deterministic owned regions.

`node --test tests/core-memory-policy-source.test.mjs` runs 63 restricted
source-only policy vectors; passing is **not** compiled Rust/Wasm guest
execution. The compiler's current module linking and real native/Wasm
execution must be separately admitted/tested before claiming usable APIs.
Maintain independent backend code, compatible semantics, and fail-closed
unsupported features. Keep draft pending genuine exact-head runner tests.

Relevant related work: [Java #1771](https://github.com/atto-lang/atto-source.java/pull/1771),
[LLVM #801](https://github.com/atto-lang/atto-source.llvm/pull/801),
[LLVM policy #873](https://github.com/atto-lang/atto-source.llvm/pull/873),
[atomic spec #175](https://github.com/atto-lang/atto-spec/pull/175),
conflicting pointer specs [#18](https://github.com/atto-lang/atto-spec/pull/18)
and [#154](https://github.com/atto-lang/atto-spec/pull/154).

These three files are source-only policy candidates. They live under
`stdlib/core/policy/` so they do not shadow the compiler-owned `core/mem`
library unit (`import module mem from "atto.core/mem"`).

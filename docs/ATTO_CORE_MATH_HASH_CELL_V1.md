# Rust/Wasm backend-local Atto core math/hash/cell candidate

Three backend-owned `.atto` modules are staged at
`library/atto.core/{math,hash,cell}.atto`. They have compatible public
math/hash/cell-policy behavior with Java #1770, LLVM #869, .NET #220
and Wasm #283, but are independently maintained in Rust.

The `tests/core_math_hash_cell_source.rs` integration test consumes actual
source file text and checks **source-free reference** golden vectors. It
does not invoke `attoc`, compile the imports, link a Wasm guest, or
synchronize any shared cell. A planned linked fixture is in
`contracts/core-math-hash-cell-v1/guest.atto`, deliberately **not** in
the accepted/executable fixture directory. The current Rust browser
compiler's single-file interface and package linkage cannot promote
these library sources to production imports until proven.

`Cell<T>` and `RefCell<T>` remain compiler-enforced design targets,
not implemented by a writable class with a fake borrow counter. The
hash fold is noncryptographic and not a secure untrusted-key hash map.
No source-only or host-Rust test should be described as real guest parity.

Promotion: parse/typecheck imports; prevent a forged `atto.core/*` source
shadow; prove exact ownership and checked scalar operators; run a real
linked Atto->Wasm guest plus negative tests on current head with nonzero
CI steps. All execution is **NOT_EXECUTED**.

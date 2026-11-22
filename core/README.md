# Rust browser Atto core slice/iter/cmp admission

The source-owned modules `core/slice.atto`, `core/iter.atto`, and
`core/cmp.atto` are **scalar-only** for this backend: range checks,
forward/reverse cursor movement, `Option<int>` checked operations,
integer/boolean ordering, min/max and clamping. They use the existing
`define pub module` and project-root `+core/name.atto` package linker.
This is a path-local package import, **not** a trusted `atto.core/name`
virtual module or an externally authenticated package registry.

Compiler admission is defined in `tests/core_slice_iter_cmp_package.rs`,
which compiles the real sources and checks deterministic Wasm bytes; it
does not by itself execute a guest. The scalar source fixture is
`core/tests/slice_iter_cmp_smoke.atto`.

The original array/callback algorithms are staged in
`core/experimental/{slice,iter,cmp}_array.atto` and the extra fixtures
`core/tests/slice_iter_cmp_edge_cases.atto` and
`core/tests/iter_callbacks_smoke.atto` are **not executable on this
browser backend yet**. In particular, no generic owned Array<T> guest
lowering, borrow-lifetime enforcement, Slice<T>/SliceMut<T>, Iterator<T>
or Ord trait dispatch is being claimed.

A general slice API must be compiler-owned borrower/owner checked and
reject escapes, invalidation by owner mutations, aliasing mutable views,
use-after-take, retained loans crossing await, and actor crossing.
No runtime allocator or host imports may be used to bypass these gates.

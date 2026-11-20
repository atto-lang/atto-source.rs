# Rust AOT physical-layout planner (prototype)

The independent Rust compiler now has a bounded, checked metadata planner for
**one authoritative physical store**. It is reusable for future data-oriented
and \`define inverted class\` lowering, but is **not yet wired to guest classes,
actors, an allocator, or a tracing JIT**. The present guest compiler still
accepts only its validated scalar subset.

\`LayoutPlan::new(kind, fields, rows)\` supports:
- **AoS:** aligned field offsets within an aligned row stride.
- **SoA:** independent, dense and aligned field columns; a 4-byte managed-ref
  column may have a 4-byte stride even when heap objects are 8-byte aligned.
- **AoSoA:** power-of-two lane blocks, independent column strides, and a fully
  reserved final partial block for stable indexing.

The planner checks field count (1–64), positive widths, power-of-two alignment,
AoSoA lanes (1–1024), integer overflow, row bounds, and field indices. Zero-row
stores have a zero-byte footprint. Offsets are relative integers only, not
native pointers. The metadata \`Vec\` allocations occur in the planning host;
they are not guest heap allocations and imply no guest tracing GC.

## Full-AOT and optional tracing/JIT boundary

AOT must remain the default and produce closed-world native code. Profiling or
a trace recorder can *suggest* AoS/SoA/AoSoA specialization, but no tracer
may mutate a live store, change its ABI, expose unproven aliasing, or bypass
ownership/actor isolation. JIT needs an explicit target capability and must be
rejected or disabled in browser CSP/freestanding/hard-RT environments that
cannot provide executable code memory. Browser output requires a separate
wasm32 code-generation path, not a native Cranelift object relabeled as Wasm.

Before guest integration: preserve typed MIR ownership, field projections,
generations and domain provenance; validate row relocation and active borrows;
add native-object execution tests for inverted fields; compare Java and LLVM
semantic fixtures; benchmark throughput/tail latency and resident bytes.
The 1,025-row / 1,024-lane tail fixture matches the exact Java reference offsets in java PR #3131. These tests verify address planning and non-overlap, **not** native guest
class execution, browser compilation, GC absence in every future runtime,
or cross-backend feature parity.

# Managed reference compression and ECS layout contract (experimental)

## Scope
ARM64 LP64 keeps 64-bit ABI pointers. Oreslang managed references are distinct and may be stored as 32-bit domain-relative encodings. ActorRef, native FFI pointers, MMIO addresses, raw pointers, and code pointers are NOT compressed by this mechanism.

## Initial ABI
- `compressed32` is a target layout mode, not a source-level type; `native` is independently supported.
- A managed region has native-width base and limit plus fixed alignment shift. Payload 0 is null; a nonzero payload N addresses `base + ((N - 1) << shift)`, subject to bounds, allocation-start, generation, lifetime and domain checks.
- With 8-byte alignment and reserved null the maximum addressable byte offset is `(2^32 - 2) * 8`. This is less than 32 GiB. Do not assume every actor allocates an enormous contiguous reservation.
- A region represents one encoding domain. Multi-region actor heaps need region-specific typing/metadata or an explicitly validated segmented format; do not silently decode foreign region refs against the current actor base.
- No fabricated cross-actor pointer: transfers use value-copy, move with ownership handoff, authorized frozen shares, or capabilities. Frozen shared regions have independent lifetime and domain identifiers.
- The codec in `src/ref_layout.rs` is arithmetic only. It does not establish allocation provenance, memory safety or borrow legality and must not be used as a standalone heap dereferencer.
- V1 fixes reference mode at build/runtime layout selection; no in-place width mutation on live heaps, and no mixed representation without specialization/adapters.
- FFI boundaries explicitly materialize native pointers under scoped pin/borrow guarantees. Escaping interior references and raw pointer reinterpretation are prohibited.

## SoA / AoSoA / ECS
- `LayoutStore<T>` selects exactly one authoritative AoS, SoA or AoSoA physical layout. No hidden mirrored copies.
- `define inverted class` retains nominal field semantics; physical SoA columns may contain compact refs or (preferably) dense integer indices.
- AoSoA fields use block+lane addressing with a per-field offset/element stride, independently of AoS size. Row relocation requires generation updates/borrow exclusions.
- ECS entities use generation-checked handles, not object pointers; component locations use dense/sparse indices, with transactional insertion and safe swap-removal.
- Do not impose 8/16 byte alignment on every SoA element just because heap *objects* are aligned. Packed `u32` reference columns remain 4-byte dense.

## MIR and backend requirements
Represent the managed-reference value as an **internal typed MIR capability**, independently of physical width. It must not be exposed as source `Ref<T, Domain>`, `Pointer<T>`, address-of, or dereference syntax. MIR carries region provenance, ownership, borrow lifetime, nullability, alignment, physical layout strategy, and allocation bounds; final lowering selects compressed or native representation.
Emit explicit `ref.encode`, `ref.decode`, bounds/provenance validation and barriers if moving objects are introduced. Native LLVM, Java/GraalVM and Rust/Cranelift must agree on semantics, not necessarily on private heap bytes.
Never assume LLVM pointer-address-space annotations alone implement compression; explicit integer storage and lowering are more portable.
Preserve hard-RT bounds: region table lookups, traps, allocation/compaction, pinning and decoding must be accounted for in WCET/effect proofs; offer fixed-size/nonmoving allocation paths.

## Differential verification gates
- Compare compressed32 and native-mode logical program results for actor-state graphs, Tasks, channels, hot reload and shared frozen data.
- Randomized encode/decode and corruption tests at 0, maximum slot, out-of-bounds, alignment, null and overflow.
- Assert actor-isolation failures, stale generation rejection, borrowed-object relocation rejection and no foreign-domain dereference.
- Cross-layout ECS tests: field-sensitive borrows, row generations, swap delete, SIMD-block boundaries, and single authoritative store.
- Benchmark resident bytes, cache misses, cycles, latency tails, and WCET for pointer-heavy actor graphs and dense SoA/AoSoA ECS. Do not assert performance gains without measurement.

## Status
This PR introduces an arithmetic reference/layout prototype, managed-layout MIR
metadata, and a standalone Cranelift codec object generator. It does not lower
guest managed objects, implement actor heaps/borrowing, or integrate compressed
references into Java or LLVM runtimes. The codec requires prior trusted bounds,
alignment, domain and lifetime validation.

## Current-main integration boundary

The existing CLI modes, bounded source reader, output-alias protections and
checked effect admission remain unchanged. Canonical effect-IR v1 and its
independent analyzer golden fixture are preserved. Optional representation
metadata is available through `export_effect_ir_with_layout_checked`, a separate
`oreslang.managed-ref-effect-envelope.v1` containing the canonical `effect_ir`.
It does not grant pointer, actor, allocation or borrowing authority.

## Bounds-checked AoSoA field addressing (new Rust stage-0 helper)

The arithmetic-only `aosoa_field_offset` cannot establish that an access
fits within a block or backing allocation. Rust now also exposes
`checked_aosoa_field_span(row, row_count, lanes, block_stride, field_base,
field_element_stride, field_width, backing_byte_len)`, returning a byte
range or a typed `RefError`. This helper rejects:

- out-of-range logical rows and accesses past the actual backing bytes;
- zero/non-power-of-two lane counts or tile widths above the canonical 1024-lane maximum, zero widths/strides and overlapping
  lane elements (field width greater than element stride);
- any layout whose **last lane** would exceed the physical block extent,
  even when the particular requested lane happens to fit;
- all checked-arithmetic overflow in field extent and row/block offsets.

It does **not** return a raw pointer, infer object lifetime, authorize a
cross-actor borrow, check field overlap with *other* columns, or prove the
backing region belongs to an actor. These obligations remain with validated
MIR and the allocator. An AoSoA layout planner must provide whole-record
nonoverlap and region-provenance proofs before native dereferencing.

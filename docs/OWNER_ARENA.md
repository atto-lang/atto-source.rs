# Bounded actor-owned byte arena (host substrate)

This Rust module is a **deterministic bump arena**, not an implemented
Oreslang guest heap. It is intended for future actor-scoped, nonmoving
allocations without tracing garbage collection. The scalar Rust frontend does
not yet admit guest actor/class allocation or ownership syntax.

## Contract

- Construction reserves up to 16 MiB of host byte storage once. Capacity
  never increases, and allocations only advance a bounded offset cursor.
- Every allocation receives an opaque region-specific, generation-stamped
  handle. A handle is not a pointer, serialized capability, send token or
  cross-actor authority.
- Handles from another arena fail even if their numeric offsets match.
  After explicit reset, all old handles fail before bytes can be accessed
  again. Arena IDs never wrap; generations never wrap.
- Reset requires exclusive mutable access, reinitializes allocated bytes
  (including padding) to zero, then returns the cursor to zero. This is
  deterministic bulk reclamation, not tracing or reference-counting.
  Memory clearance is for safe reuse, **not cryptographic secure erasure**.
- Allocations and reads/writes are bounds-checked; capacity, lengths,
  offset alignments, additions and epoch increments fail closed.
- The backing allocation is a byte buffer. Aligned offsets do **not**
  establish aligned native pointers, FFI validity or typed object placement.
  No pointer casting is authorized or implemented.
- The runtime must establish trusted actor identity, scheduling exclusivity,
  lifetime quiescence, mailbox/Task ownership and capability policy before
  exposing these operations to actual Oreslang programs.
- A temporarily borrowed byte slice is scoped by Rust borrowing; reset
  cannot run while the borrow remains active in safe Rust.

## Evidence

The Rust tests cover unique arenas, foreign handles, reset and stale leases,
zeroed reuse, nonmoving allocation offsets, invalid sizes/alignments, bounded
capacity, failure atomicity, epoch exhaustion and concurrent distinct arenas.

This work alone is **not** proof of no GC in the Java, LLVM, Wasm, .NET or Rust
guest runtimes. No full AOT guest allocation is emitted and no real actor
message crosses an arena boundary. Future implementation must integrate
typed MIR borrow rules, generation/owner-aware actor handles, exact guest
source execution, cross-backend conformance, and bounded resource/latency
evidence.

## Browser / native / desktop

The implementation uses std byte storage and a global pointer-sized atomic
to mint unique local arena IDs. Each target must explicitly demonstrate
support for required atomics and allocation at construction. Browser
wasm32, desktop, freestanding and hard-real-time targets are **not yet
certified**. In a no-allocator target the arena should instead accept
caller-provided, statically reserved storage under a separately reviewed
provenance and ownership model.

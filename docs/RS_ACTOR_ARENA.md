# Rust actor-owned fixed-capacity arena — stage 0

`src/actor_arena.rs` is a small host-side foundation for zero-GC actor
storage. It contains exactly `N` inline slots and never invokes a
garbage collector or dynamically grows its storage during `insert`,
`get`, `get_mut` or `take`. The stored value `T` may independently
own heap allocations; this module does not promise a whole-program
no-allocation guarantee.

## Proven properties

- Slot handles are opaque Rust values, not host pointers or ActorRefs.
- The owner actor and incarnation are checked on every access. A
  different incarnation rejects old handles after an actor restart.
- Each slot has an independent, monotonically increasing generation.
  `take` advances it, and overflow permanently retires a slot instead
  of letting a stale generation wrap and regain authority.
- All admission is bounded by a compile-time capacity. Exhaustion
  returns `CapacityExhausted`, never silently evicts an object.
- `get_mut` follows Rust's exclusive mutable borrowing rules within
  one arena instance. Removing a middle slot does not move neighbors.
- No `unsafe` code, pointer arithmetic, hidden mirroring or runtime
  code generation is introduced.

## Out of scope

This is **not** a secure capability system or complete zero-GC Oreslang
runtime. Actor IDs are supplied by the caller and not cryptographic;
the supervisor must guarantee unique, nonzero incarnation IDs.
The module does not validate source `rt borrow`, `rt take`, `critical-read`
or `critical-write`, integrate compressed references, transmit values
across trust domains, check stack bounds, enforce actor scheduling or
prove hard-real-time deadlines. Actor lifecycle and cross-thread
ownership still require MIR and runtime enforcement.

The code introduces an owned slot store, not an implicit permission to
reconstruct or dereference a native pointer from an integer handle.
The `src/ref_layout.rs` compressed32 codec remains arithmetic-only and
needs independent domain, allocation-start and lifetime validation.

## Validation

```sh
cargo fmt --all -- --check
cargo test --locked --all-targets
cargo clippy --locked --all-targets -- -D warnings
```

Rust regression tests exercise owner/incarnation mismatch, capacity,
stale handles after reuse, generation saturation, stable neighboring
slots, mutable borrowing, duplicate removal, zero capacity and
non-Copy ownership transfer. Full native guest and Java/LLVM parity
tests must be added after owned MIR/actor lowering exists.

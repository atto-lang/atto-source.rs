# Experimental emitted Wasm lease-cleanup witness

This milestone is a **real executable Wasm instruction-level release witness**,
not a compiler for user-written stateful `mtd` callback bodies.

The opt-in `attoc --cleanup-wasm-witness input.atto -o witness.wasm`
and browser compiler entrypoint `compile_cleanup_wasm_witness(length)`
accept exactly one typed receiver with exactly one validated
`critical-write` callback containing potentially checked arithmetic.
The source frontend validates the callback, and
`lower_verified_cleanup_cfg` verifies all required release obligations.
That CFG can **only** select the fixed synthetic witness program; it does
not translate the source callback's state expressions.

## Guest witness ABI

The generated module has no imports or memory and exports:

- `run_cleanup_witness(i64 delta, i32 cancel) -> i64`: acquire a private,
  simulated exclusive lease; cancel when `cancel != 0`; otherwise perform a
  checked signed-64-bit addition to a private initial value of 41
- `lease_active() -> i32`: 0 when no simulated lease is held
- `acquire_count() -> i32` and `release_count() -> i32`: monotonic counters
- `state_value() -> i64`: private synthetic value, not the source actor field
- `close_witness() -> i32`: terminally close when no lease is active
- `panic_code` mutable i32 global: 1 for checked overflow, 7 for call after
  closure or invalid internal acquisition

The emitter constructs three bounded branches. Every branch has a distinct
actual Wasm instruction sequence that clears `lease_active` and increments
`release_count` **before** its terminal operation:

1. **Normal completion:** commit the checked sum, release, return its value
2. **Checked overflow:** release, set `panic_code=1`, execute `unreachable`
3. **Cooperative cancellation:** release, return -1 without changing value

The separate `audit_cleanup_wasm_witness` function scans the **actual emitted
Wasm code section**, verifies the function count and complete run-body
instruction sequence, and rejects missing, doubled, moved or mutated releases.
Malformed or caller-forged frontend proof flags cannot authorize this emitter.
The production `require_backend_cleanup_proof` gate remains *unchanged* and
rejects all stateful `mtd` methods. Native/wasm32 compiler byte parity and
actual Node WebAssembly execution are tested by
`tests/target_cleanup_wasm.mjs`.

## Scope and limits

This does **not** prove cleanup for general AttoLang callback IR, multiple
leases, suspension, error propagation, unwind across arbitrary generated
instructions, or real actor-state fields. The witness has a synthetic
Wasm-private lease, so terminating the entire Wasm instance discards that
private state; it does **not** release a separate host-owned lock. Production
cancellation, watchdog termination and actor-generation fencing require
host-supervisor lease ownership or generation-scoped invalidation, verified
independently before stateful compilation is enabled.

The previous `--scalar-preview` and `--receiver-instance-preview` outputs and
their browser receiver ABI are unchanged.

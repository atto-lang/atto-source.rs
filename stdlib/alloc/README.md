# Atto-owned allocator policy modules

These are implementation files **in AttoLang**. Each compiler repository owns
its own copy; cross-backend source does not have to be byte-identical.

* `alloc/aligned`: checked power-of-two padding, over-allocation plans and
  page counts. Requires an authenticated allocator to produce *physically*
  aligned pinned storage; integer offsets alone do not establish that fact.
* `alloc/stats`: bounded per-owner accounting ledger and fragmentation inputs.
  Real heap usage, leak findings, frees and allocation identities must originate
  in authenticated allocator receipts, never caller-supplied counters.
* `alloc/secure`: logical in-place clearing and closed-buffer state, **not**
  secret-key-safe erasure. `secure_clear` deliberately rejects until the
  backend implements guaranteed non-elidable zeroization and ownership/lifetime
  controls, including on failure and cancellation paths.

The fixture in `tests/fixtures/alloc_foundations_smoke.atto` specifies the
intended public behavior. It is a guest execution *candidate* and must not be
reported as passing until source module resolution, compiler lowering and
actual execution are verified on the exact head. Java, LLVM and Rust/Wasm may
have different runtime providers and implementation details.

Use `atto.alloc/<module>` for unambiguous imports. Unsupported backend
capabilities must fail closed; do not silently lower secure memory to ordinary
array clearing or treat logical padding as a physical alignment proof.

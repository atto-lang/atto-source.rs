# Experimental actor job frame revision 0

**Status:** internal Rust transport codec candidate; NOT normative, NOT a
compiled guest feature and NOT a public AttoLang ABI. This source is stacked on
[atto-source.rs#310](https://github.com/atto-lang/atto-source.rs/pull/310).
It is independent of the source parser/checker and is not called by the
production Wasm emitter or runtime.

`src/job_envelope.rs` round-trips an immutable
`(namespace, job_id, attempt, correlation_id, signed i64 payload)` with exact
UTF-8 key identity and explicit fixed little-endian lengths. It rejects
truncation, extra bytes, corrupt UTF-8, invalid revision, reserved flags,
empty fields, oversized keys and forged lengths. The full frame is bounded to
**4,380 bytes**: a 28-byte header, a combined 4,096-byte namespace/job ID
budget and a 256-byte correlation ID budget.

The byte layout is:

| Offset | Field | Encoding |
|---|---|---|
| 0 | Magic | `ATJB` |
| 4 | Revision | `0` |
| 5 | Reserved flags | `0` |
| 6 | Namespace length | u16 little-endian |
| 8 | Job ID length | u16 little-endian |
| 10 | Correlation ID length | u16 little-endian |
| 12 | Attempt | u64 little-endian |
| 20 | Scalar payload | i64 little-endian |
| 28 | Namespace, job ID, correlation ID bytes | contiguous exact UTF-8 |

**No permission or provenance assertion is made from a parsed frame.**
A frame is untrusted data, not an authenticated ActorRef, job receipt,
incarnation token, tenant capability or proof of mailbox admission.
No guest receives these bytes today. Host-side job ID idempotency, durable
storage and authenticated completion receipts are separate protocols.

Before adopting any normative v1 format, require independent review of
[atto-spec#113](https://github.com/atto-lang/atto-spec/pull/113),
the ownership and copying rules, host-to-guest memory placement/reading,
capability checks, endian/canonicalization rules, job attempts, correlation ID
limits, encryption/authentication where needed and a guest import/export ABI
version bump. The old `actor_receive(type, value, sequence)` cannot magically
consume this frame and must not be presented as equivalent.

Testing `rustc --edition=2021 --test src/job_envelope.rs` exercises the
pure codec only. A passing codec test is not a compiler-backend conformance pass.

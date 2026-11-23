# Rust allocation parity: two compiler-negative witnesses

The Rust compiler already has integration tests for `drop! x; drop! x`
(`E_MOVED`) and for dropping a class owner while `borrow! owner`
remains live (`E_BORROW`). This PR exposes two independently hashed
Atto-source fixtures with a dedicated `cargo test` harness. A passing
workflow may emit `ATTO_ALLOC_CASE` after the genuine Atto compiler
rejected each fixture with the expected diagnostic.

The test does **not** execute an Atto guest or assert physical allocation
and destructor behavior. No `ATTO_ALLOC_GUEST_EXECUTED` marker is
produced. These are **2/22** compiler-rejection obligations under the
shared conformance contract; all others remain NOT_VERIFIED.

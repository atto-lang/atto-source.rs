# HTTP routing guest compilation baseline

Tracks atto-source.rs issue #348 and atto-http-routing PR #2.

A self-contained Atto file-local module in
`tests/fixtures/pass/174-routing-scalar-wasm.atto` returns an independently
verified exit value of 42. Rust test `tests/routing_guest_baseline.rs` calls
the real Atto compiler, checks deterministic WebAssembly generation and executes
the produced guest in Node with fail-closed host effects.

This is a **scalar-only compiler milestone**, not execution of the production
HTTP router. It covers exact string-key value comparisons, one static route,
a coarse string-prefix route, and path-first method rejection. It has no
Array<T>, generic capture decoding, trie, URL parser, cross-file import,
resource authorization, or actor transport.

The negative checks must fail without guest artifacts for Array literals,
String.replace_literal and unlinked external module imports. They are
explicit, current compiler blockers, not passing production route behavior.

To run:

```bash
cargo test --locked --test routing_guest_baseline -- --nocapture
```

Requires Rust and Node.js. Do not promote atto-http-routing PR #2 to ready until
the five actual .atto router guest fixtures pass against a pinned compiler,
and the requested backends have independent execution evidence.

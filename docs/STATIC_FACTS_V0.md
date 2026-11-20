# Common advisory static facts v0 — Rust Cranelift

The `oresc --static-facts input.ores` command runs the real Rust frontend
parser then `mir::lower` (the same semantic admission used for object code).
It emits `ores-static-facts/v0` with exact UTF-8 SHA-256 source identity,
canonical `__root__.name/arity` procedure IDs and unproved direct calls.

No general actor/Task, async, borrow, FFI, loops or callbacks exist in this
small admitted frontend. Zero loops here are a **closed grammar observation**,
not a claim that arbitrary Oreslang code terminates. Repeated source
callsites with the same caller/callee fail closed until callsite IDs and
multiplicity are available. The adapter also refuses more than 512 functions,
8192 distinct direct edges, 128 expression traversal levels or 30000 nodes.
Source hashes are NOT attestations of trusted producer identity. No numeric
work/allocation/copy proof is emitted. The pure Rust SHA-256 helper has known
empty/abc regression vectors to avoid requiring a new Cargo.lock dependency.

Run `cargo test --locked --all-targets` and
`cargo run --locked -- --static-facts tests/fixtures/forward.ores`.
Only executed Actions steps constitute test evidence; unsigned JSON is never
hard-real-time, strict-AOT or security admission. See shared contract PR #106
and restricted Rust analyzer PR #16.

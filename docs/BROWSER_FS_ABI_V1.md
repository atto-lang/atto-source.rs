# Browser filesystem ABI admission candidate (v1)

**Status:** typed Rust **admission/reference model**, not compiled Atto filesystem execution.

This branch adds the host/compiler-side lexical and ownership contract for a future guest WASM filesystem request/resume ABI, parallel to the existing typed browser HTTP work in [atto-source.rs #386](https://github.com/atto-lang/atto-source.rs/pull/386).

- Host import module: atto_fs_v1, with read_request / write_request / cancel.
- Guest export names: atto_fs_v1_reserve and atto_fs_v1_resume.
- Wasm32 wire values: i32 positions, lengths, ids, numeric tags; do not expose raw OS descriptors or a JS host object as a guest value.
- Request IDs are positive and bound to a host-validated tenant + actor incarnation. READ and WRITE are independent grants, not inferred from a path string.
- Path validation accepts only UTF-8, bounded tenant-virtual relative names; rejects absolute paths, traversal and invalid UTF-8. This does NOT replace safe dirfd or handle-rooted checks for real mounts.
- Guest memory offsets and lengths require checked overflow-safe bounds and a copy/pin before any suspend. The Rust helper validates ranges but does not access Wasm memory or implement pinning.
- A finite pending-request table enforces duplicate, quota, settle-once, and irrevocable revocation. No guest import is emitted by this change.

The companion development-stage host adapter lives in [atto-source.wasm #43](https://github.com/atto-lang/atto-source.wasm/pull/43). Its actual loopback HTTP + memory-VFS host test is not a compiled Atto guest test. The normative language proposal and cross-backend acceptance are [atto-spec #117](https://github.com/atto-lang/atto-spec/pull/117) and [atto-platform-conformance #101](https://github.com/atto-lang/atto-platform-conformance/pull/101).

To test the Rust contract when a Rust toolchain and repo are available:

    cargo test --locked --test browser_fs_abi_v1

**Toolchain limitation in this work session:** rustc/cargo are unavailable locally; this test has been added but has **not executed**. GitHub CI must prove it on the exact PR head, with nonzero steps. No network, file, or guest Wasm operation is performed by these Rust tests.

Next: implement the native Atto std.fs public module and linked package import, finish susp/await lowering, authenticate the guest's byte snapshot and continuation ticket, add the corresponding Wasm host broker import under an explicit trusted capability grant, and prove real guest HTTP → filesystem roundtrip with exact-commit receipts. Until then any attempted Atto source call to std.fs must fail closed with no guest artifact.

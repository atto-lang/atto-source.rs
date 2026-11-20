# Rust/browser-Wasm canonical ByteBuffer integration checkpoint

**Branch:** feat/byte-buffer-runtime-v1-20261119 (draft PR #417)

**Status:** source admission and lexer/parser compatibility implemented. The actual canonical std.buffer class is NOT compiled, linked or guest executed by the ordinary Rust compiler.

## Implemented

- src/std_buffer_gate.rs pins SHA-256, byte length, and canonical paths for the exact four atto-common-source #30 candidate modules. A successful verifier yields immutable compiler-owned copies. Caller mutations cannot replace a previously verified source.
- Test-only source snapshots under tests/fixtures/canonical-buffer-v1 are exact source Git blobs, not host implementations.
- Ordinary compile(), compile_with_imports(), and linker source graph parsing reject std.buffer with E_STDLIB_UNLINKED before any Wasm output or host fallback. Package-string equality is not considered authorization.
- The existing normal lexer now parses bounded hexadecimal 0x i64 literals; the parser accepts throws Error (beside throws(Error)) and explicit val mut. Signed range and malformed literals remain fail-closed.
- Tests exercise actual ordinary scalar compiler-to-Wasm emission, lexing and negative effect admission, candidate identity, source tampering and caller-mutation isolation.
- .github/workflows/byte-buffer-compiler-gate.yml pins the exact PR commit, checks source blobs and runs Rust 1.85.1 regression and Clippy gates.

## Remaining blockers to full M1/M2

1. The existing checked linker/package resolver must load the authenticated versioned std.buffer package and enforce module visibility. It cannot substitute a host class merely because the package string matches.
2. The ordinary Rust parser/type checker must handle the complete canonical ByteBuffer source: constructor, static factory, sealed OwnedBytes field/intrinsic, imported endian/bounds helpers, mutable receivers and Error subclasses. The syntax-only changes are necessary but not sufficient.
3. The sealed runtime intrinsic must make owner/tenant/generation authority unforgeable and enforce aggregate quotas, exclusive mutable loans, generation fencing, zero initialization and safe cancellation. Host-provided arbitrary owner metadata is not a security boundary.
4. Typed BufferBoundsError, BufferValueError, BufferAllocationError, TenantQuotaError and fail-atomic runtime mutations must be implemented and checked.
5. Compile and execute shared BB-001 through the ordinary Rust compiler. Acceptance: guest returns 1 and contains 78 56 34 12 00 00 00 00. Then execute all 22 positive/negative cases from atto-platform-conformance #104.

The separate atto-source.wasm #47 experimental compiler executes 11 positive .atto programs but does not parse/link the canonical class and cannot substitute for these gates.

## Exact-head CI verification (not yet executed)

    cargo +1.85.1 test --locked --lib std_buffer_gate -- --nocapture
    cargo +1.85.1 test --locked --lib
    cargo +1.85.1 clippy --locked --lib -- -D warnings

Source organization GitHub Actions jobs previously returned zero/null steps; these are NOT_EXECUTED rather than test passes. No private-source cross-org mirror may be performed without security authorization. Keep PR #417 Draft and do not claim compiler parity or secure TenantVM until the full guest tests, exact-head CI, review, and branch-protection conditions pass.

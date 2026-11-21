# Experimental native LLVM AOT adapter

**Canonical frontend:** `atto-lang/atto-source.rs` (this repository's root
`atto-compiler-browser` crate). Do not place an independent copy of its
`lexer.rs`, `parser.rs`, `check.rs`, `ast.rs` or `wasm.rs` here.

This draft moves the *Rust-authored LLVM IR emitter* out of
`atto-lang/atto-source.llvm` without porting its duplicate Rust browser
compiler. The existing C++/LLVM compiler remains under `atto-source.llvm`.
This adapter produces LLVM native AOT (not WebAssembly) by consuming this
repository's checked Rust IR. The C actor runtime here is a bounded
Linux/POSIX experimental host component; it is not a secure production TenantVM.

## Provenance, original source blobs

- `src/main.rs` from `atto-source.llvm@79827f2a4ccc2a66c218135b14e480e7be9ad2fe`, Git blob `2a8f6f587dbd4d86ac346b72589dc4c5a7f394b2`.
- `src/target_native.rs` from the same commit, Git blob `d9ba4ecb57c08f141613fbeff9705803927f7e6e`.
- `runtime/actor_runtime.c` from the same commit, Git blob `95780f61ed1deba59a2375710a25eee6e9e5b86a`.

**Compatibility delta:** The relocated `src/main.rs` now explicitly rejects
six `BinOp` variants added since the frozen frontend snapshot
(`BitAnd`, `BitOr`, `BitXor`, `Shl`, `LShr`, `AShr`) with
`E_UNSUPPORTED_NATIVE_BITWISE`, rather than retaining a non-exhaustive
Rust match against the canonical AST. The SHA above records the **original**
source blob, not the post-fix blob. The migrated LLVM emitter also now
validates the complete 64-byte ELF64 header, ET_EXEC/ET_DYN, x86-64 machine,
nonempty entrypoint, in-file program headers and executable mode. A successful
compiler process with a forged 20-byte ELF magic prefix cannot publish an
output artifact. The native smoke runner includes a malicious zero-exit
`clang` test that must preserve any previously published bytes. These
checks are format sanity gates, **not** signing, executable authenticity,
tenant isolation, or proof of native bitwise support.

**Note:** The 2026-11-14 old compiler relied on a pinned frontend snapshot.
The canonical root compiler has evolved since then. Byte-identity of the unchanged target linker and C actor runtime, and
recorded provenance of the now-modified emitter, do **not** prove they compile
against the current frontend.
No build, guest execution, or ABI equivalence claim is made until real
exact-head CI executes.

## Build candidate

From `atto-source.rs`:

```sh
cargo +1.85.1 test --locked --manifest-path experimental/llvm-aot/Cargo.toml --offline
cargo +1.85.1 build --locked --release --manifest-path experimental/llvm-aot/Cargo.toml --offline
```

A successful build must be followed by native Clang executable integration
and all existing `atto-source.llvm/ci/test_native_aot.sh` and
`ci/test_native_actors.sh` guest cases adapted to this path, including
checked arithmetic failures, output safety and actor isolation checks.

**Important:** This subtree is a migration candidate, not a new canonical
backend. It must not publish/compiler-export binaries or relax private-source
CI permissions. The deletion of the vendored frontend in `.llvm` is
dependent on exact-head native guest verification and review of its existing
`cargo` entrypoints and workflow consumers.

## Executable gates (once an admitted runner is available)

The isolated package now pins its own `Cargo.lock`, and the root-source
read-only GitHub workflow `.github/workflows/llvm-aot-adapter.yml`
runs `experimental/llvm-aot/ci/verify-native-guest.sh`.
The script requires a pinned exact source HEAD, native `cargo +1.85.1`
build/test, deterministic LLVM IR, a real ELF64 x86-64 native guest
returning exit code 42, fail-closed invalid source with no new artifact,
and C actor runtime compilation with warnings as errors.

**Verification status:** shell syntax and gate-presence assertions were
checked locally on the authored script; Rust 1.85.1 is unavailable in the
working container, and current source GitHub jobs fail before runner steps.
No successful Cargo test, native guest execution or independent carrier run
is claimed. Cross-org source exports remain denied by the root
`.github/private-ci-approved-repositories.txt`; source `main` has
not been independently verified as protected. A source-free Actions
capacity retry on `zed-pkg-test`, `athlet-o` and `voxletra` returned
three further zero-step failures, not usable runner receipts.

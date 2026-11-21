#!/usr/bin/env bash
# Atto source.rs: real Rust-native + Rust-wasm32 compiler -> Atto guest Wasm.
# The test must run on the exact PR head; no JS source compiler is accepted.
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd -P)"
cd "$root"
if [[ -n "${EXPECTED_SOURCE_SHA:-}" ]]; then
  [[ "$EXPECTED_SOURCE_SHA" =~ ^[0-9a-f]{40}$ ]]
  test "$(git rev-parse HEAD)" = "$EXPECTED_SOURCE_SHA"
  git diff --quiet HEAD --
  git diff --cached --quiet HEAD --
  test "$(git write-tree)" = "$(git rev-parse 'HEAD^{tree}')"
  test -z "$(git ls-files --others --exclude-standard)"
fi
command -v cargo
command -v node
node --version
rustc +1.85.1 --version
rustup target list --installed --toolchain 1.85.1 | grep -Fxq wasm32-unknown-unknown
export CARGO_NET_OFFLINE=true
# Use the site's current reproducible optimization profile; fail closed if
# the compiled compiler exceeds the 1-MiB web-distribution size budget.
export CARGO_PROFILE_RELEASE_OPT_LEVEL=z
cargo +1.85.1 test --locked --offline --lib
cargo +1.85.1 build --locked --offline --release --bin attoc
cargo +1.85.1 rustc --locked --offline --release --lib \
  --target wasm32-unknown-unknown -- -C link-arg=--max-memory=67108864
compiler="$root/target/wasm32-unknown-unknown/release/atto_compiler.wasm"
test -s "$compiler"
actual_bytes="$(wc -c < "$compiler")"
if (( actual_bytes > 1048576 )); then
  echo "E_COMPILER_SIZE: Rust browser compiler exceeds 1 MiB: $actual_bytes bytes" >&2
  exit 1
fi
export ATTO_RUST_WASM_COMPILER="$compiler"
export ATTO_RUST_NATIVE_CLI="$root/target/release/attoc"
node --test tests/rust_browser_compiler_e2e.mjs
sha256sum "$compiler" "$ATTO_RUST_NATIVE_CLI"
echo "PASS: exact Rust compiler source -> real wasm32 compiler -> real executed Atto guest Wasm"

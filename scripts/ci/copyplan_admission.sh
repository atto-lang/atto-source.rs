#!/usr/bin/env bash
# Reproducible bounded CopyPlan admission checks. Intended to run in the
# source checkout OR in a byte-identical, Git-tree-authenticated CI mirror.
# This script NEVER checks out a different organization or uses a GitHub token.
set -euo pipefail
cd "$(dirname "$0")/../.."
TOOLCHAIN="${ATTO_COPYPLAN_RUST_VERSION:-1.85.1}"
command -v node >/dev/null
command -v python3 >/dev/null
cargo +"$TOOLCHAIN" test --locked --all-targets
cargo +"$TOOLCHAIN" build --locked --release --bin attoc
cargo +"$TOOLCHAIN" rustc --locked --release --lib \
  --target wasm32-unknown-unknown --crate-type cdylib -- \
  -C link-arg=--max-memory=67108864
test -s target/wasm32-unknown-unknown/release/atto_compiler.wasm
ATTOC=target/release/attoc node tests/copy_doublebang_guest.mjs
ATTOC=target/release/attoc node tests/conformance.mjs
ATTOC=target/release/attoc node tests/callable_guest_execution.mjs
ATTOC=target/release/attoc node tests/defer_recover_guest.mjs
echo "COPYPLAN_REQUIRED_COMPILERS_AND_GUESTS=PASS"
# Strict whole-project Clippy remains red in unrelated legacy modules.
# Allow only the exact known set of pre-existing warnings; fail on new ones.
tmp="$(mktemp)"
trap 'rm -f "$tmp"' EXIT
cargo +"$TOOLCHAIN" clippy --locked --all-targets \
  --message-format=json -- -D warnings > "$tmp" 2>/dev/null || :
python3 scripts/ci/guard_copyplan_clippy.py < "$tmp"

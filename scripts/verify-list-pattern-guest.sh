#!/usr/bin/env bash
# Exact-head source-compiled List<int> proof. Do not use host-reference
# test results as a substitute for executing compiler-generated Wasm.
set -euo pipefail

cd "$(dirname "${BASH_SOURCE[0]}")/.."

if [[ ! "${ATTO_EXPECT_HEAD:-}" =~ ^[0-9a-f]{40}$ ]]; then
  echo 'ATTO_EXPECT_HEAD must be the exact 40-digit PR source SHA' >&2
  exit 2
fi
actual_head="$(git rev-parse HEAD)"
if [[ "$actual_head" != "$ATTO_EXPECT_HEAD" ]]; then
  printf 'SOURCE_SHA_MISMATCH expected=%s actual=%s\n' "$ATTO_EXPECT_HEAD" "$actual_head" >&2
  exit 2
fi
if [[ -n "$(git status --porcelain)" ]]; then
  echo 'SOURCE_TREE_DIRTY: verification requires a clean exact-head checkout' >&2
  exit 2
fi

command -v cargo >/dev/null || { echo 'MISSING_CARGO: no tests were executed' >&2; exit 3; }
command -v rustc >/dev/null || { echo 'MISSING_RUSTC: no tests were executed' >&2; exit 3; }
command -v node >/dev/null || { echo 'MISSING_NODE: no guest execution was performed' >&2; exit 3; }

printf 'SOURCE_HEAD=%s\n' "$actual_head"
printf 'SOURCE_TREE=%s\n' "$(git rev-parse 'HEAD^{tree}')"
rustc --version
cargo --version
node --version

# Run material tests before constructing the guest. Every command must pass.
cargo test --locked \
  --test list_pattern_guest \
  --test typed_seq_fragment_admission \
  --test sequence_v2_fail_closed \
  --test sequence_v2_ellipsis_lexer
cargo clippy --locked --all-targets -- -D warnings
cargo build --locked --release --bin attoc
ATTOC="$(pwd)/target/release/attoc" node tests/list-pattern-guest-execution.mjs

printf 'ATTO_LIST_PATTERN_VERIFIED=PASS source_head=%s source_tree=%s\n' \
  "$actual_head" "$(git rev-parse 'HEAD^{tree}')"

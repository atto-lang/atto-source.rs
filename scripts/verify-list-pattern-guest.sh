#!/usr/bin/env bash
# Exact-head source-compiled List<int> proof. Do not use host-reference
# test results as a substitute for executing compiler-generated Wasm.
set -euo pipefail

cd "$(dirname "${BASH_SOURCE[0]}")/.."

if [[ ! "${ATTO_EXPECT_HEAD:-}" =~ ^[0-9a-f]{40}$ ]]; then
  echo 'ATTO_EXPECT_HEAD must be the exact 40-digit PR source SHA' >&2
  exit 2
fi
# A private carrier necessarily creates a different commit SHA even if it
# mirrors every path, mode, and file byte exactly. Accept that carrier ONLY when
# the independently pinned original source Git tree matches the entire checkout
# tree. In normal mode, require the original commit SHA as before.
mode="${ATTO_VERIFY_MODE:-source}"
if [[ "$mode" != source && "$mode" != mirror ]]; then
  echo 'INVALID_VERIFY_MODE: expected source or mirror' >&2
  exit 2
fi
actual_head="$(git rev-parse HEAD)"
actual_tree="$(git rev-parse 'HEAD^{tree}')"
expected_tree="${ATTO_EXPECT_TREE:-}"
if [[ -n "$expected_tree" && ! "$expected_tree" =~ ^[0-9a-f]{40}$ ]]; then
  echo 'INVALID_SOURCE_TREE_SHA: expected 40 lowercase hexadecimal digits' >&2
  exit 2
fi
if [[ "$mode" == source && "$actual_head" != "$ATTO_EXPECT_HEAD" ]]; then
  printf 'SOURCE_SHA_MISMATCH expected=%s actual=%s\n' "$ATTO_EXPECT_HEAD" "$actual_head" >&2
  exit 2
fi
if [[ "$mode" == mirror && -z "$expected_tree" ]]; then
  echo 'MIRROR_REQUIRES_PINNED_SOURCE_TREE: set ATTO_EXPECT_TREE from the original source commit' >&2
  exit 2
fi
if [[ -n "$expected_tree" && "$actual_tree" != "$expected_tree" ]]; then
  printf 'SOURCE_TREE_MISMATCH expected=%s actual=%s\n' "$expected_tree" "$actual_tree" >&2
  exit 2
fi
if [[ -n "$(git status --porcelain)" ]]; then
  echo 'SOURCE_TREE_DIRTY: verification requires a clean checkout' >&2
  exit 2
fi
# Ignored in-tree inputs are invisible to git status but could influence a
# Cargo build (e.g. an ignored .cargo/config.toml, generated source, or target/).
# Require a genuinely fresh checkout for an admissible executable proof.
if [[ -n "$(git clean -ndx)" ]]; then
  echo 'SOURCE_EXTRA_IGNORED_PATHS: fresh checkout required (git clean -ndx lists extra files)' >&2
  exit 2
fi
# Git-tree object identity does not attest to unmaterialized gitlink contents.
# This narrow proof path does not fetch/validate submodule repositories.
if git ls-tree -r HEAD | grep -E '^160000 commit ' >/dev/null; then
  echo 'SOURCE_SUBMODULE_UNVERIFIED: gitlinks require a separate pinned proof' >&2
  exit 2
fi

printf 'VERIFY_MODE=%s\nEXPECTED_SOURCE_HEAD=%s\nCHECKOUT_HEAD=%s\nSOURCE_TREE=%s\n' \
  "$mode" "$ATTO_EXPECT_HEAD" "$actual_head" "$actual_tree"
if [[ "$mode" == mirror ]]; then
  echo 'MIRROR_PROVENANCE=tree-identical; commit identity intentionally differs'
fi

command -v cargo >/dev/null || { echo 'MISSING_CARGO: no tests were executed' >&2; exit 3; }
command -v rustc >/dev/null || { echo 'MISSING_RUSTC: no tests were executed' >&2; exit 3; }
command -v node >/dev/null || { echo 'MISSING_NODE: no guest execution was performed' >&2; exit 3; }

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

printf 'ATTO_LIST_PATTERN_VERIFIED=PASS verify_mode=%s pinned_source_head=%s checkout_head=%s source_tree=%s\n' \
  "$mode" "$ATTO_EXPECT_HEAD" "$actual_head" "$actual_tree"

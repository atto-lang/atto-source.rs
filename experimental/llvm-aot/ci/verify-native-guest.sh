#!/usr/bin/env bash
# Native ELF guest conformance for the single-source Rust frontend + LLVM emitter.
# Run on exact atto-source.rs PR head after runner admission.
set -euo pipefail
root="$(cd "$(dirname "$0")/../../.." && pwd -P)"
cd "$root"
manifest="$root/experimental/llvm-aot/Cargo.toml"
command -v cargo >/dev/null
command -v clang >/dev/null
command -v readelf >/dev/null
command -v timeout >/dev/null
clang --version | head -n 1
if [[ -n "${EXPECTED_SOURCE_SHA:-}" ]]; then
  [[ "$EXPECTED_SOURCE_SHA" =~ ^[0-9a-f]{40}$ ]]
  test "$(git rev-parse HEAD)" = "$EXPECTED_SOURCE_SHA"
  git diff --quiet HEAD --
  git diff --cached --quiet HEAD --
  test "$(git write-tree)" = "$(git rev-parse 'HEAD^{tree}')"
  test -z "$(git ls-files --others --exclude-standard)"
  test -z "$(git ls-files --others --ignored --exclude-standard)"
fi
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
export CARGO_TARGET_DIR="$work/target"
export CARGO_NET_OFFLINE=true
cargo +1.85.1 test --locked --offline --manifest-path "$manifest"
cargo +1.85.1 build --locked --offline --release --manifest-path "$manifest" --bin atto-llvm
compiler="$CARGO_TARGET_DIR/release/atto-llvm"
test -x "$compiler"
cat > "$work/positive.atto" <<'ATTO'
pub fnc main(): int { return 42; }
ATTO
"$compiler" "$work/positive.atto" -o "$work/positive.ll" --emit-llvm
"$compiler" "$work/positive.atto" -o "$work/positive-again.ll" --emit-llvm
cmp "$work/positive.ll" "$work/positive-again.ll"
grep -q 'define i32 @main()' "$work/positive.ll"
"$compiler" "$work/positive.atto" -o "$work/positive.exe"
readelf -h "$work/positive.exe" > "$work/elf-header.txt"
grep -q 'Class:[[:space:]]*ELF64' "$work/elf-header.txt"
set +e
timeout -k 3 20 "$work/positive.exe"
status=$?
set -e
if (( status != 42 )); then
  echo "LLVM_ADAPTER_GUEST_FAILED: expected native exit 42, got $status" >&2
  exit 1
fi
cat > "$work/negative.atto" <<'ATTO'
pub fnc main(): int { return not_bound; }
ATTO
printf 'keep-original-output\n' > "$work/negative.ll"
if "$compiler" "$work/negative.atto" -o "$work/negative.ll" --emit-llvm; then
  echo 'LLVM_ADAPTER_UNSAFE: invalid source compiled' >&2
  exit 1
fi
test "$(cat "$work/negative.ll")" = 'keep-original-output'
if "$compiler" "$work/negative.atto" -o "$work/negative.exe"; then
  echo 'LLVM_ADAPTER_UNSAFE: invalid source produced an executable' >&2
  exit 1
fi
test ! -e "$work/negative.exe"
clang -std=c11 -O2 -pthread -Wall -Wextra -Werror -c \
  "$root/experimental/llvm-aot/runtime/actor_runtime.c" -o "$work/actor_runtime.o"
sha256sum "$compiler" "$work/positive.ll" "$work/positive.exe"
echo 'PASS: typed frontend -> LLVM native AOT; deterministic IR; exit-42 guest; fail-closed negative; C actor compile'

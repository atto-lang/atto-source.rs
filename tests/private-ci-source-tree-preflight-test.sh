#!/usr/bin/env bash
# Fully synthetic temporary Git repositories; no private source or network.
set -euo pipefail
preflight=$(realpath -- "$(dirname -- "$0")/private-ci-source-tree-preflight.sh")
scratch=$(mktemp -d)
trap 'rm -rf -- "$scratch"' EXIT
repo="$scratch/fixtures"
mkdir -p "$repo"
git -C "$repo" init -q
git -C "$repo" config user.name 'Synthetic CI Fixture'
git -C "$repo" config user.email 'synthetic@example.invalid'
printf 'example\n' > "$repo/example.txt"
git -C "$repo" add example.txt
git -C "$repo" commit -qm 'synthetic baseline'
head=$(git -C "$repo" rev-parse HEAD)
tree=$(git -C "$repo" rev-parse 'HEAD^{tree}')
passed=0
expect_allow() {
  local name=$1
  if ! (cd "$repo" && bash "$preflight" "$head" "$tree") > "$scratch/out" 2> "$scratch/err"; then
    printf 'FAIL: expected accept for %s\n' "$name" >&2
    cat "$scratch/err" >&2
    exit 1
  fi
  grep -q '^PRIVATE_CI_SOURCE_PREFLIGHT_OK ' "$scratch/out"
  passed=$((passed + 1))
}
expect_deny() {
  local name=$1
  if (cd "$repo" && bash "$preflight" "$head" "$tree") > "$scratch/out" 2> "$scratch/err"; then
    printf 'FAIL: expected reject for %s\n' "$name" >&2
    exit 1
  fi
  grep -q '^PRIVATE_CI_SOURCE_PREFLIGHT_REJECTED:' "$scratch/err" || {
    printf 'FAIL: missing diagnostic for %s\n' "$name" >&2
    cat "$scratch/err" >&2
    exit 1
  }
  passed=$((passed + 1))
}
expect_allow 'clean baseline'
head_actual=$head
tree_actual=$tree
head=0000000000000000000000000000000000000000
expect_deny 'incorrect source head'
head=$head_actual
tree=0000000000000000000000000000000000000000
expect_deny 'incorrect tree root'
tree=$tree_actual
printf 'changed\n' >> "$repo/example.txt"
expect_deny 'modified tracked file'
git -C "$repo" restore example.txt
printf 'extra\n' > "$repo/untracked.txt"
expect_deny 'untracked file'
rm "$repo/untracked.txt"
printf 'secret\n' > "$repo/ignored.tmp"
printf 'ignored.tmp\n' > "$repo/.gitignore"
git -C "$repo" add .gitignore
git -C "$repo" commit -qm 'add ignored rule'
head=$(git -C "$repo" rev-parse HEAD)
tree=$(git -C "$repo" rev-parse 'HEAD^{tree}')
expect_deny 'ignored file'
rm "$repo/ignored.tmp"
expect_allow 'clean with gitignore'
ln -s example.txt "$repo/linked.txt"
git -C "$repo" add linked.txt
git -C "$repo" commit -qm 'add synthetic symlink'
head=$(git -C "$repo" rev-parse HEAD)
tree=$(git -C "$repo" rev-parse 'HEAD^{tree}')
expect_deny 'symlink Git mode'
git -C "$repo" rm -q linked.txt
git -C "$repo" commit -qm 'remove symlink'
head=$(git -C "$repo" rev-parse HEAD)
tree=$(git -C "$repo" rev-parse 'HEAD^{tree}')
printf 'version https://git-lfs.github.com/spec/v1\noid sha256:0000\nsize 1\n' > "$repo/pointer.dat"
git -C "$repo" add pointer.dat
git -C "$repo" commit -qm 'synthetic LFS pointer'
head=$(git -C "$repo" rev-parse HEAD)
tree=$(git -C "$repo" rev-parse 'HEAD^{tree}')
expect_deny 'LFS pointer'
rm -f "$repo/pointer.dat"
git -C "$repo" add -u
git -C "$repo" commit -qm 'remove pointer'
head=$(git -C "$repo" rev-parse HEAD)
tree=$(git -C "$repo" rev-parse 'HEAD^{tree}')
expect_allow 'restored after pointer'
printf 'changed\n' > "$repo/example.txt"
git -C "$repo" add example.txt
expect_deny 'staged modification'
git -C "$repo" restore --staged --worktree example.txt
expect_allow 'restored after staged modification'
original_head=$head
head=not-a-git-sha
expect_deny 'malformed source head'
head=$original_head
printf '#!/bin/sh\nexit 0\n' > "$repo/runner.sh"
chmod +x "$repo/runner.sh"
git -C "$repo" add runner.sh
git -C "$repo" commit -qm 'valid executable mode'
head=$(git -C "$repo" rev-parse HEAD)
tree=$(git -C "$repo" rev-parse 'HEAD^{tree}')
expect_allow 'regular executable mode'
git -C "$repo" rm -q runner.sh
git -C "$repo" commit -qm 'remove executable'
head=$(git -C "$repo" rev-parse HEAD)
tree=$(git -C "$repo" rev-parse 'HEAD^{tree}')
weird_name=$'tab\tand newline\nfile.txt'
printf 'content\n' > "$repo/$weird_name"
git -C "$repo" add -- "$weird_name"
git -C "$repo" commit -qm 'nonstandard but valid Git path'
head=$(git -C "$repo" rev-parse HEAD)
tree=$(git -C "$repo" rev-parse 'HEAD^{tree}')
expect_allow 'NUL-delimited tab and newline Git path'
git -C "$repo" rm -q -- "$weird_name"
git -C "$repo" commit -qm 'remove weird path'
head=$(git -C "$repo" rev-parse HEAD)
tree=$(git -C "$repo" rev-parse 'HEAD^{tree}')
git -C "$repo" update-index --add --cacheinfo "160000,$head,synthetic_gitlink"
git -C "$repo" commit -qm 'synthetic gitlink entry'
head=$(git -C "$repo" rev-parse HEAD)
tree=$(git -C "$repo" rev-parse 'HEAD^{tree}')
expect_deny 'gitlink Git mode'
echo "PASS: $passed synthetic read-only source preflight cases"

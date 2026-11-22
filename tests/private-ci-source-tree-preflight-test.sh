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
{
  printf 'version https://git-lfs.github.com/spec/v1\n'
  printf 'ext-foo '
  head -c 1200 /dev/zero | tr '\000' a
  printf '\noid sha256:%064d\nsize 42\n' 0
} > "$repo/pointer.dat"
git -C "$repo" add pointer.dat
git -C "$repo" commit -qm 'synthetic oversized LFS header'
head=$(git -C "$repo" rev-parse HEAD)
tree=$(git -C "$repo" rev-parse 'HEAD^{tree}')
expect_deny 'oversized LFS-looking blob'
rm -f "$repo/pointer.dat"
git -C "$repo" add -u
git -C "$repo" commit -qm 'remove pointer'
head=$(git -C "$repo" rev-parse HEAD)
tree=$(git -C "$repo" rev-parse 'HEAD^{tree}')
expect_allow 'restored after pointer'
head -c 2048 /dev/zero | tr '\000' b > "$repo/large-normal.bin"
git -C "$repo" add large-normal.bin
git -C "$repo" commit -qm 'synthetic large regular blob'
head=$(git -C "$repo" rev-parse HEAD)
tree=$(git -C "$repo" rev-parse 'HEAD^{tree}')
expect_allow 'normal large blob without LFS header'
git -C "$repo" rm -q large-normal.bin
git -C "$repo" commit -qm 'remove synthetic large blob'
head=$(git -C "$repo" rev-parse HEAD)
tree=$(git -C "$repo" rev-parse 'HEAD^{tree}')
printf 'changed\n' > "$repo/example.txt"
git -C "$repo" add example.txt
expect_deny 'staged modification'
git -C "$repo" restore --staged --worktree example.txt
expect_allow 'restored after staged modification'
git -C "$repo" update-index --assume-unchanged example.txt
printf 'tampered by assume-unchanged\n' > "$repo/example.txt"
expect_deny 'dirty source hidden by assume-unchanged'
git -C "$repo" update-index --no-assume-unchanged example.txt
git -C "$repo" restore example.txt
git -C "$repo" update-index --skip-worktree example.txt
printf 'tampered by skip-worktree\n' > "$repo/example.txt"
expect_deny 'dirty source hidden by skip-worktree'
git -C "$repo" update-index --no-skip-worktree example.txt
git -C "$repo" restore example.txt
git -C "$repo" config core.filemode false
chmod +x "$repo/example.txt"
expect_deny 'executable permission hidden by core.filemode false'
chmod -x "$repo/example.txt"
git -C "$repo" config core.filemode true
expect_allow 'clean after adversarial Git index flags'
external_hardlink="$scratch/external-hardlink.txt"
ln "$repo/example.txt" "$external_hardlink"
expect_deny 'tracked source linked outside checkout'
rm -f -- "$external_hardlink"
expect_allow 'clean after external hardlink removed'
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

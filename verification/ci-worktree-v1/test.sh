#!/usr/bin/env bash
# Execute in an isolated temporary Git repository; never modify source checkout.
set -euo pipefail
here=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)
verify="$here/verify.sh"
[[ -f "$verify" ]] || { echo 'missing verifier' >&2; exit 1; }
tmp=$(mktemp -d)
trap 'rm -rf -- "$tmp"' EXIT
cd "$tmp"
git init -q
git config user.name 'CI fixture'
git config user.email 'ci-fixture@example.invalid'
printf 'base\n' > file.txt
printf 'ignored/\n' > .gitignore
git add file.txt .gitignore
git commit -qm 'fixture base'
head=$(git rev-parse HEAD)
pass=0
expect_pass(){
  if ! bash "$verify" "$head" >/dev/null 2>&1; then echo "FAIL expected admission: $1" >&2; exit 1; fi
  pass=$((pass+1)); echo "PASS: $1"
}
expect_deny(){
  if bash "$verify" "$2" >/dev/null 2>&1; then echo "FAIL expected denial: $1" >&2; exit 1; fi
  pass=$((pass+1)); echo "PASS: $1"
}
restore(){ git reset --hard -q HEAD; git clean -ffdx -q; }
expect_pass 'clean exact HEAD'
expect_deny 'incorrect SHA' "$(printf '0%.0s' {1..40})"
expect_deny 'malformed SHA' 'bad-commit'
printf 'unstaged\n' > file.txt
expect_deny 'unstaged tracked mutation' "$head"; restore
printf 'staged\n' > file.txt
git add file.txt
expect_deny 'staged tracked mutation' "$head"
git show HEAD:file.txt > file.txt
expect_deny 'staged-only index mutation, clean worktree' "$head"; restore
printf 'extra\n' > extra.txt
expect_deny 'untracked file' "$head"; restore
mkdir ignored
printf 'shadow\n' > ignored/generated.o
expect_deny 'ignored file' "$head"; restore
: > $'\n'
expect_deny 'newline-only untracked filename' "$head"; restore
GIT_INDEX_FILE="$tmp/alternate-index" bash "$verify" "$head" >/dev/null 2>&1 && { echo 'FAIL index override accepted' >&2; exit 1; }
pass=$((pass+1)); echo 'PASS: override index refused'
expect_pass 'clean checkout after all cases'
printf 'Verifier local Git fixtures: %s passed\n' "$pass"

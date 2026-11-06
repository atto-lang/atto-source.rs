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
printf 'literal case source\n' > source.atto
printf '#!/bin/sh\n' > task.sh
ln -s source.atto link
chmod 644 task.sh
git add file.txt .gitignore source.atto task.sh link
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
printf 'hidden change\n' > file.txt
git update-index --assume-unchanged file.txt
expect_deny 'assume-unchanged hides a modified tracked file' "$head"
git update-index --no-assume-unchanged file.txt; restore
git update-index --skip-worktree file.txt
expect_deny 'skip-worktree index marker on clean file' "$head"
git update-index --no-skip-worktree file.txt; restore
chmod 755 task.sh
git config core.filemode false
expect_deny 'filemode=false hides tracked executable-bit mutation' "$head"
git config core.filemode true; chmod 644 task.sh; restore
rm link; printf 'source.atto' > link
git config core.symlinks false
expect_deny 'symlinks=false hides plain file replacing tracked symlink' "$head"
git config core.symlinks true; rm link; ln -s source.atto link; restore
printf 'shadow path\n' > SOURCE.atto
git config core.ignorecase true
expect_deny 'ignorecase=true hides case-colliding source file' "$head"
git config core.ignorecase false; restore
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

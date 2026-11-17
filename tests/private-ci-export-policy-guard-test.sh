#!/usr/bin/env bash
# Self-contained synthetic regression cases; never reads or exports source code.
set -euo pipefail
cd "$(dirname "$0")/.."
guard=scripts/private-ci-export-policy-guard.sh
[[ -f "$guard" ]] || { echo 'missing policy guard' >&2; exit 1; }
workspace=$(mktemp -d)
trap 'rm -rf -- "$workspace"' EXIT
file="$workspace/policy.txt"
pass=0

expect_allow() {
  local name=$1 payload=$2
  rm -f -- "$file"
  printf '%b' "$payload" > "$file"
  if ! bash "$guard" "$file" > "$workspace/out" 2> "$workspace/err"; then
    echo "FAIL: $name should be admitted as comment-only" >&2
    cat "$workspace/err" >&2
    exit 1
  fi
  pass=$((pass + 1))
}

expect_deny() {
  local name=$1 payload=$2
  rm -f -- "$file"
  printf '%b' "$payload" > "$file"
  if bash "$guard" "$file" > "$workspace/out" 2> "$workspace/err"; then
    echo "FAIL: $name accepted forbidden input" >&2
    exit 1
  fi
  grep -q '^PRIVATE_CI_EXPORT_REJECTED:' "$workspace/err" || {
    echo "FAIL: $name missing failure diagnostic" >&2; exit 1;
  }
  pass=$((pass + 1))
}

expect_allow 'empty' ''
expect_allow 'comments and whitespace' '# none\n   \n\t# still denied\n'
expect_allow 'CRLF comments' '# Windows comment\r\n'
expect_deny 'active destination' 'foreign/repo 123\n'
expect_deny 'active after comment' '# comment\nforeign/repo 123\n'
expect_deny 'NUL-prefix binary grep bypass' '\000# comment\n'
expect_deny 'NUL after comment' '# comment\000foreign/repo 123\n'
expect_deny 'ESC comment' '# comment\033\n'
expect_deny 'DEL comment' '# comment\177\n'
expect_deny 'UTF-8 nonbreaking space' '\302\240# comment\n'
expect_deny 'active CRLF' 'foreign/repo 123\r\n'
expect_deny 'vertical tab' '#comment\v\n'
expect_deny 'backspace' '#comment\b\n'
expect_deny 'invalid UTF8' '#comment\377\n'

# The inclusive 64 KiB boundary must not be rejected spuriously.
rm -f "$file"
printf '#' > "$file"
head -c 65535 /dev/zero | tr '\000' a >> "$file"
bash "$guard" "$file" > "$workspace/out" 2> "$workspace/err" || {
  echo 'FAIL: exactly 65536 comment-only bytes denied' >&2; cat "$workspace/err" >&2; exit 1;
}
pass=$((pass + 1))
printf 'a' >> "$file"
if bash "$guard" "$file" > "$workspace/out" 2> "$workspace/err"; then
  echo 'FAIL: 65537 bytes allowed' >&2; exit 1
fi
pass=$((pass + 1))

printf '# comment\n' > "$workspace/target"
rm -f "$file"
ln -s "$workspace/target" "$file"
if bash "$guard" "$file" > "$workspace/out" 2> "$workspace/err"; then
  echo 'FAIL: symlink leaf admitted' >&2; exit 1
fi
pass=$((pass + 1))

mkdir "$workspace/actual"
printf '# comment\n' > "$workspace/actual/policy"
ln -s "$workspace/actual" "$workspace/aliased"
if bash "$guard" "$workspace/aliased/policy" > "$workspace/out" 2> "$workspace/err"; then
  echo 'FAIL: symlink parent admitted' >&2; exit 1
fi
pass=$((pass + 1))

rm -f "$file"
printf '# comment\n' > "$file"
ln "$file" "$workspace/hardlink"
if bash "$guard" "$file" > "$workspace/out" 2> "$workspace/err"; then
  echo 'FAIL: hardlinked policy admitted' >&2; exit 1
fi
pass=$((pass + 1))
rm -f "$workspace/hardlink"

rm -f "$file"
mkdir "$file"
if bash "$guard" "$file" > "$workspace/out" 2> "$workspace/err"; then
  echo 'FAIL: directory admitted' >&2; exit 1
fi
pass=$((pass + 1))
rmdir "$file"
mkfifo "$file"
if timeout 3 bash "$guard" "$file" > "$workspace/out" 2> "$workspace/err"; then
  echo 'FAIL: FIFO admitted' >&2; exit 1
fi
pass=$((pass + 1))
rm -f "$file"
if bash "$guard" "$file" > "$workspace/out" 2> "$workspace/err"; then
  echo 'FAIL: missing policy admitted' >&2; exit 1
fi
pass=$((pass + 1))

printf 'PASS: %d deny-by-default private CI policy regression cases\n' "$pass"

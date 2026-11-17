#!/usr/bin/env bash
# Synthetic inputs only. No private repository content or credentials.
set -euo pipefail
cd "$(dirname "$0")/.."
guard=scripts/private-ci-export-policy-guard.sh
[[ -f "$guard" ]] || { echo 'missing policy guard' >&2; exit 1; }
workspace=$(mktemp -d)
trap 'rm -rf -- "$workspace"' EXIT
file="$workspace/policy.txt"
pass=0

allow() {
  local case_name=$1 payload=$2
  printf '%b' "$payload" > "$file"
  if ! bash "$guard" "$file" > "$workspace/out" 2> "$workspace/err"; then
    echo "FAIL: \${case_name}: comment-only policy rejected" >&2
    cat "$workspace/err" >&2
    exit 1
  fi
  pass=$((pass + 1))
}

deny() {
  local case_name=$1 payload=$2
  printf '%b' "$payload" > "$file"
  if bash "$guard" "$file" > "$workspace/out" 2> "$workspace/err"; then
    echo "FAIL: \${case_name}: forbidden policy accepted" >&2
    exit 1
  fi
  grep -q '^PRIVATE_CI_EXPORT_REJECTED:' "$workspace/err" || {
    echo "FAIL: \${case_name}: no fail-closed diagnostic" >&2
    exit 1
  }
  pass=$((pass + 1))
}

allow 'empty policy' ''
allow 'comments and whitespace' '# approved destinations: none\n   \n\t# still denied\n'
allow 'CRLF comments' '# Windows comment\r\n'
deny 'active destination' 'foreign/repo 123\n'
deny 'active destination after comment' '# allowed comment\nforeign/repo 123\n'
deny 'NUL-prefix grep bypass' '\000# comment\n'
deny 'NUL inside comment' '# comment\000foreign/repo 123\n'
deny 'ESC inside comment' '# comment\033\n'
deny 'DEL inside comment' '# comment\177\n'
deny 'UTF8 nonbreaking-space comment' '\302\240# comment\n'
deny 'active destination with CRLF' 'foreign/repo 123\r\n'

printf '# ' > "$file"
head -c 65536 /dev/zero | tr '\000' a >> "$file"
if bash "$guard" "$file" > "$workspace/out" 2> "$workspace/err"; then
  echo 'FAIL: oversized comment-only policy accepted' >&2
  exit 1
fi
pass=$((pass + 1))

printf '# comment\n' > "$workspace/target"
rm -f "$file"
ln -s "$workspace/target" "$file"
if bash "$guard" "$file" > "$workspace/out" 2> "$workspace/err"; then
  echo 'FAIL: symlink policy accepted' >&2
  exit 1
fi
pass=$((pass + 1))
rm -f "$file"
if bash "$guard" "$file" > "$workspace/out" 2> "$workspace/err"; then
  echo 'FAIL: missing policy accepted' >&2
  exit 1
fi
pass=$((pass + 1))

printf 'PASS: %d deny-by-default private CI policy regressions\n' "$pass"

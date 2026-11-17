#!/usr/bin/env bash
# Deny export unless the policy is a bounded ASCII, comment-only regular file.
# This guard does NOT authorize a destination. A separate independently reviewed
# mechanism is required before any private source may leave the repository.
set -euo pipefail

reject() {
  printf 'PRIVATE_CI_EXPORT_REJECTED: %s\n' "$1" >&2
  exit 1
}

[[ $# -eq 1 ]] || reject 'expected exactly one policy-file path'
file=$1
[[ -f "$file" && ! -L "$file" ]] || reject 'policy file absent, non-regular or symlinked'
bytes=$(stat -c '%s' -- "$file") || reject 'policy file size unreadable'
[[ "$bytes" =~ ^[0-9]+$ && "$bytes" -le 65536 ]] || reject 'policy file exceeds 65536 bytes'

# GNU grep defaults to binary-file heuristics; a NUL before a comment can
# cause grep -Evq to return no non-comment matches. Reject all non-ASCII and
# control bytes (apart from TAB, CR, LF) BEFORE interpreting comment lines.
clean=$(mktemp) || reject 'unable to allocate policy validation buffer'
trap 'rm -f -- "$clean"' EXIT
LC_ALL=C tr -cd '\011\012\015\040-\176' < "$file" > "$clean" || reject 'policy byte validation failed'
cmp -s -- "$file" "$clean" || reject 'policy contains binary, control or non-ASCII bytes'

# Treat grep errors as denials, never as evidence that the policy is empty.
set +e
LC_ALL=C grep -aEvq '^[[:space:]]*(#.*)?$' -- "$file"
classification=$?
set -e
case "$classification" in
  0) reject 'active private-source destination needs independently reviewed authorization' ;;
  1) ;;
  *) reject 'policy classification failed' ;;
esac

printf '%s\n' 'private CI source export: disabled by default'

#!/usr/bin/env bash
# Deny private source export unless the policy is a bounded ASCII, comment-only
# regular file. This guard never authorizes a cross-organization destination.
set -euo pipefail

reject() {
  printf 'PRIVATE_CI_EXPORT_REJECTED: %s\n' "$1" >&2
  exit 1
}

[[ $# -eq 1 ]] || reject 'expected exactly one policy-file path'
file=$1
[[ -f "$file" && ! -L "$file" ]] || reject 'policy file absent, non-regular or symlinked'

# Reject symlinked parent components, not just a symlink at the leaf.
resolved=$(realpath -e -- "$file") || reject 'policy path cannot be resolved'
logical=$(realpath -e -s -- "$file") || reject 'policy path cannot be normalized'
[[ "$resolved" == "$logical" ]] || reject 'policy path contains a symlinked component'

# Hold one open descriptor while taking a private, immutable validation snapshot.
# This prevents the byte validator and classifier from opening different files.
exec {policy_fd}< "$file" || reject 'policy file cannot be opened'
fd_path="/proc/self/fd/$policy_fd"
[[ -f "$fd_path" ]] || reject 'opened policy is not a regular file'
link_count=$(stat -Lc '%h' -- "$fd_path") || reject 'policy link count unreadable'
[[ "$link_count" == 1 ]] || reject 'hardlinked policy file forbidden'
bytes=$(stat -Lc '%s' -- "$fd_path") || reject 'policy file size unreadable'
[[ "$bytes" =~ ^[0-9]+$ && "$bytes" -le 65536 ]] || reject 'policy file exceeds 65536 bytes'
opened_identity=$(stat -Lc '%d:%i' -- "$fd_path") || reject 'opened file identity unreadable'
path_identity=$(stat -c '%d:%i' -- "$file") || reject 'policy path identity unreadable'
[[ "$opened_identity" == "$path_identity" ]] || reject 'policy changed before validation'

scratch=$(mktemp -d) || reject 'unable to allocate policy validation directory'
trap 'rm -rf -- "$scratch"' EXIT
snapshot="$scratch/policy"
clean="$scratch/ascii-only"
cat <&"$policy_fd" > "$snapshot" || reject 'policy snapshot failed'
exec {policy_fd}<&-
[[ $(stat -c '%s' -- "$snapshot") == "$bytes" ]] || reject 'policy changed while reading'
path_identity_after=$(stat -c '%d:%i' -- "$file") || reject 'policy disappeared while reading'
[[ "$path_identity_after" == "$opened_identity" ]] || reject 'policy replaced while reading'

# GNU grep's binary-file heuristic can wrongly admit NUL-prefixed comments.
# Reject controls and non-ASCII before checking that every line is a comment.
LC_ALL=C tr -cd '\011\012\015\040-\176' < "$snapshot" > "$clean" || reject 'byte validation failed'
cmp -s -- "$snapshot" "$clean" || reject 'binary, control or non-ASCII policy input'

# grep 0: active entry; 1: comments/whitespace only; 2+: operational failure.
set +e
LC_ALL=C grep -aEvq '^[[:space:]]*(#.*)?$' -- "$snapshot"
classification=$?
set -e
case "$classification" in
  0) reject 'active destination requires independently reviewed authorization' ;;
  1) ;;
  *) reject 'policy classifier failed' ;;
esac
printf '%s\n' 'private CI source export: disabled by default'

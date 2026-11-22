#!/usr/bin/env bash
# Read-only, source-local integrity preflight for a separately authorized CI mirror.
# This script never copies source, changes Git refs, or approves any destination.
set -euo pipefail

reject() {
  printf 'PRIVATE_CI_SOURCE_PREFLIGHT_REJECTED: %s\n' "$1" >&2
  exit 1
}

[[ $# -eq 2 ]] || reject 'expected exact source commit SHA and root tree SHA'
expected_head=$1
expected_tree=$2
[[ "$expected_head" =~ ^[0-9a-f]{40}$ ]] || reject 'invalid expected commit SHA'
[[ "$expected_tree" =~ ^[0-9a-f]{40}$ ]] || reject 'invalid expected root tree SHA'
[[ "$(git rev-parse --show-object-format=storage 2>/dev/null)" == sha1 ]] || reject 'requires SHA-1 Git object format'
root=$(git rev-parse --show-toplevel 2>/dev/null) || reject 'not in a Git working tree'
cd -- "$root"
[[ "$(git rev-parse --verify 'HEAD^{commit}')" == "$expected_head" ]] || reject 'unexpected source commit'
[[ "$(git rev-parse --verify 'HEAD^{tree}')" == "$expected_tree" ]] || reject 'unexpected root Git tree'

git diff --quiet --exit-code HEAD -- || reject 'tracked working tree differs from HEAD'
git diff --cached --quiet --exit-code HEAD -- || reject 'Git index differs from HEAD'

scratch=$(mktemp -d) || reject 'cannot allocate private inspection directory'
trap 'rm -rf -- "$scratch"' EXIT
manifest="$scratch/manifest"
other="$scratch/untracked-or-ignored"
ignored="$scratch/ignored"
lfs_magic="$scratch/lfs-magic"
printf '%s' 'version https://git-lfs.github.com/spec/v1' > "$lfs_magic"
git ls-files --others --exclude-standard -z > "$other" || reject 'untracked-file scan failed'
[[ ! -s "$other" ]] || reject 'untracked files in source checkout'
git ls-files --others --ignored --exclude-standard -z > "$ignored" || reject 'ignored-file scan failed'
[[ ! -s "$ignored" ]] || reject 'ignored files in source checkout'
git ls-tree -r -z --full-tree HEAD > "$manifest" || reject 'Git tree inventory failed'

count=0
while IFS= read -r -d '' record; do
  mode=${record%% *}
  remainder=${record#* }
  kind=${remainder%% *}
  remainder=${remainder#* }
  [[ "$remainder" == *$'\t'* ]] || reject 'malformed Git tree record'
  oid=${remainder%%$'\t'*}
  path=${remainder#*$'\t'}
  [[ "$mode" == 100644 || "$mode" == 100755 ]] || reject 'symlink, gitlink or unknown file mode'
  [[ "$kind" == blob && "$oid" =~ ^[0-9a-f]{40}$ && -n "$path" ]] || reject 'invalid path, type or blob SHA'
  [[ "$path" != /* && "$path" != ../* && "$path" != *'/../'* ]] || reject 'unsafe source path'

  actual_oid=$(git cat-file blob "$oid" | git hash-object --stdin) || reject 'blob unavailable or unreadable'
  [[ "$actual_oid" == "$oid" ]] || reject 'Git blob content hash mismatch'
  # Independently hash checkout bytes: assume-unchanged and skip-worktree
  # index flags can conceal modified source files from git diff.
  [[ -f "$path" && ! -L "$path" ]] || reject 'missing or symlinked checked-out source file'
  working_oid=$(git hash-object --no-filters -- "$path") || reject 'checked-out source file unreadable'
  [[ "$working_oid" == "$oid" ]] || reject 'checked-out source bytes differ from committed blob'
  case "$mode" in
    100644) [[ ! -x "$path" ]] || reject 'unexpected checked-out executable permission' ;;
    100755) [[ -x "$path" ]] || reject 'missing checked-out executable permission' ;;
  esac

  # Verify LFS-looking prefix at all sizes: an oversized pointer must fail.
  # Complete Git object content was hashed above; ignore only early SIGPIPE
  # from the prefix check, not the independent full-content verification.
  if (set +o pipefail; git cat-file blob "$oid" | head -c 42 | cmp -s -- "$lfs_magic" -); then
    reject 'Git LFS pointer requires separate source classification'
  fi
  count=$((count + 1))
done < "$manifest"
[[ "$count" -gt 0 ]] || reject 'empty source tree'

printf 'PRIVATE_CI_SOURCE_PREFLIGHT_OK head=%s tree=%s blobs=%d manifest_sha256=%s\n' \
  "$expected_head" "$expected_tree" "$count" "$(sha256sum "$manifest" | cut -d ' ' -f 1)"

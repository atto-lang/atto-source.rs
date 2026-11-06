#!/usr/bin/env bash
# Source-only exact-head and physical checkout admission. Run before compiler tests.
set -euo pipefail
fail() { printf 'EXACT_SOURCE_DENIED: %s\n' "$1" >&2; exit 1; }
[[ $# -eq 1 && "$1" =~ ^[0-9a-f]{40}$ ]] || fail 'expected one full lower-case source commit SHA'
# Do not let caller-controlled Git plumbing redirect the identity/index being verified.
[[ -z "${GIT_DIR:-}" && -z "${GIT_WORK_TREE:-}" && -z "${GIT_INDEX_FILE:-}" ]] || fail 'Git path/index override is not allowed'
root=$(git -c core.fsmonitor=false rev-parse --show-toplevel) || fail 'Git checkout is unavailable'
[[ "$(pwd -P)" == "$(cd "$root" && pwd -P)" ]] || fail 'must run from repository root'
[[ "$(git -c core.fsmonitor=false rev-parse --verify HEAD)" == "$1" ]] || fail 'checkout HEAD differs from expected source commit'
[[ "$(git -c core.fsmonitor=false rev-parse --verify 'HEAD^{tree}')" =~ ^[0-9a-f]{40}$ ]] || fail 'cannot identify source tree'
# Refuse Git assume-unchanged and skip-worktree index bits: either can hide
# modified tracked content from ordinary diff/status even at the correct HEAD.
index_flags=$(mktemp) || fail 'cannot allocate index-flags receipt'
trap 'rm -f -- "$index_flags"' EXIT
git -c core.fsmonitor=false ls-files -v -z > "$index_flags" || fail 'cannot enumerate Git index flags'
[[ -s "$index_flags" ]] || fail 'empty tracked Git index'
while IFS= read -r -d '' indexed; do
  [[ "${indexed:0:2}" == 'H ' ]] || fail 'nonordinary assume-unchanged or skip-worktree index flag'
done < "$index_flags"
# Both comparisons are essential: the index may be dirty even if the worktree
# has been restored to HEAD, and a plain unstaged diff does not check that case.
git -c core.fsmonitor=false diff --no-ext-diff --no-textconv --cached --quiet HEAD -- || fail 'staged/index changes detected'
git -c core.fsmonitor=false diff --no-ext-diff --no-textconv --quiet -- || fail 'unstaged tracked changes detected'
# NUL delimiters and full byte counts catch hostile newline-only filenames;
# never use shell command substitution on the filenames themselves.
unknown=$(git -c core.fsmonitor=false ls-files --others --exclude-standard -z | wc -c) || fail 'cannot enumerate untracked files'
[[ "$unknown" -eq 0 ]] || fail 'untracked checkout files detected'
ignored=$(git -c core.fsmonitor=false ls-files --others --ignored --exclude-standard -z | wc -c) || fail 'cannot enumerate ignored files'
[[ "$ignored" -eq 0 ]] || fail 'ignored checkout shadow files detected'
printf 'EXACT_SOURCE_OK head=%s tree=%s\n' "$1" "$(git -c core.fsmonitor=false rev-parse 'HEAD^{tree}')"

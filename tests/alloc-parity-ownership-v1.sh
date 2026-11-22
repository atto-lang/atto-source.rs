#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

# Run the production Atto->Wasm compiler's integration tests on actual .atto
# fixture bytes. A parser-only or host string grep is not counted as proof.
cargo test --locked --test alloc_parity_ownership_v1 -- --nocapture 2>&1 | tee "$tmp/rust.log"
grep -Eq 'test ownership_double_drop_rejected[[:space:]]+\.\.\.[[:space:]]+ok' "$tmp/rust.log"
grep -Eq 'test borrow_drop_while_live_read_rejected[[:space:]]+\.\.\.[[:space:]]+ok' "$tmp/rust.log"
count=0
while IFS=$'\t' read -r case_id phase outcome path diagnostic; do
  [[ -z "$case_id" || "$case_id" == \#* ]] && continue
  test -f "$path"
  test "$phase" = "compile" && test "$outcome" = "REJECT"
  case "$case_id:$diagnostic" in
    ownership.double_drop:E_MOVED|borrow.drop_while_live_read:E_BORROW) ;;
    *) echo "unapproved semantic case mapping" >&2; exit 1;;
  esac
  sha="$(git hash-object "$path")"
  test "$(git rev-parse "HEAD:$path")" = "$sha"
  printf 'ATTO_ALLOC_CASE %s %s %s %s\n' "$case_id" "$phase" "$outcome" "$sha"
  count=$((count+1))
done < tests/fixtures/alloc-parity-v1/rust-cases.tsv
test "$count" -eq 2
echo ATTO_RS_ALLOC_COMPILE_NEGATIVE_2_EXECUTED
# Deliberately do NOT print ATTO_ALLOC_GUEST_EXECUTED: both are compiler
# rejection tests, not an executed allocation guest or a full 22-case gate.

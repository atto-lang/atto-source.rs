#!/usr/bin/env bash
# Measure host compiler builds, not Atto guest compilation latency.
set -euo pipefail
root=$(mktemp -d)
trap 'rm -rf -- "$root"' EXIT
rustc --version
cargo --version
cargo fetch --locked
for trial in 1 2 3 4; do
  profiles=(dev dev-fast)
  if (( trial % 2 == 0 )); then profiles=(dev-fast dev); fi
  for profile in "${profiles[@]}"; do
    export CARGO_TARGET_DIR="$root/$trial/$profile"
    for phase in clean noop; do
      echo "HOST_BUILD_TIMING trial=$trial profile=$profile phase=$phase"
      /usr/bin/time -f 'elapsed_seconds=%e peak_rss_kib=%M' -o "$root/time" cargo build --locked --profile "$profile" --timings --bin attoc
      cat "$root/time"
      read -r elapsed rss < "$root/time"
      printf '%s\t%s\t%s\t%s\t%s\n' "$trial" "$profile" "$phase" "${elapsed#*=}" "${rss#*=}" >> "$root/results.tsv"
    done
  done
  if (( trial != 4 )); then rm -rf -- "$root/$trial"; fi
done
for profile in dev dev-fast; do
  for phase in clean noop; do
    for column in 4 5; do
      median=$(awk -F '\t' -v p="$profile" -v s="$phase" -v c="$column" '$2==p && $3==s {print $c}' "$root/results.tsv" | sort -n | awk 'NR==2 {a=$1} NR==3 {b=$1} END {if(NR!=4) exit 1; print (a+b)/2}')
      metric=elapsed_seconds
      if (( column == 5 )); then metric=peak_rss_kib; fi
      echo "HOST_BUILD_MEDIAN profile=$profile phase=$phase $metric=$median"
    done
  done
done
export CARGO_TARGET_DIR="$root/4/dev-fast"
cargo test --locked --profile dev-fast --test multihop_linker --test core_option_result_scalar --test incremental_build

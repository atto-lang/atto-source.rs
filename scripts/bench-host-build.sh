#!/usr/bin/env bash
# Compare host compiler build cost, not the speed of compiling Atto guests.
set -euo pipefail
root=$(mktemp -d)
trap 'rm -rf -- "$root"' EXIT
rustc --version
cargo --version
# Both profiles start with separate empty target directories, on the same
# runner/toolchain. Download dependencies once before timed compilation.
cargo fetch --locked
for profile in dev dev-fast; do
  export CARGO_TARGET_DIR="$root/$profile"
  echo "HOST_BUILD_TIMING profile=$profile phase=clean"
  /usr/bin/time -f 'elapsed_seconds=%e peak_rss_kib=%M' cargo build --locked --profile "$profile" --timings --bin attoc
  echo "HOST_BUILD_TIMING profile=$profile phase=noop"
  /usr/bin/time -f 'elapsed_seconds=%e peak_rss_kib=%M' cargo build --locked --profile "$profile" --timings --bin attoc
done
# Reuse the dev-fast artifacts for actual tests; no release/safety profile change.
export CARGO_TARGET_DIR="$root/dev-fast"
cargo test --locked --profile dev-fast --test multihop_linker --test core_option_result_scalar

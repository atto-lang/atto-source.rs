# Legacy native structural-depth regression

The browser Cargo package intentionally only declares `attoc`. It cannot
build the legacy `oresc` executable through Cargo's automatic integration-test
discovery. The original native object and diagnostic cases are preserved here.

Run them against an independently built compatible native compiler:

```sh
rustc --edition=2021 --test tests/legacy/structural_ast_depth_cli.rs -o /tmp/oresc-structural-depth-test
ORESC_BIN=/absolute/path/to/oresc /tmp/oresc-structural-depth-test
```

This is a separate execution profile, not evidence of passing native tests.
The ordinary browser crate runs the actual attoc CLI with check and Wasm emission
modes in `tests/structural_ast_depth_cli.rs`.

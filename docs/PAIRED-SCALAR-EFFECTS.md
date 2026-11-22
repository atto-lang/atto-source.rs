# Paired current-main Rust scalar compiler evidence

`oresc --compile-with-effects source.ores out.o out.json` checks and lowers
**one source instance to one MIR**, rejects effects that exceed the finite
64-level / 100,000-operation export admission limits, and uses that exact MIR
for both Cranelift object code and effect-IR v1.

The independently maintained analyzer fixture
`oreslang-static-analysis.rs/fixtures/rust-forward.effects.json` is mirrored
in `tests/fixtures/forward.effects.json` and compared by a Rust unit test
and by an executable native CI gate on `tests/fixtures/forward.ores`.

This is a **scalar prototype**: it includes checked local variables,
arithmetic and resolved same-module calls. It cannot prove target WCET,
bounded stack, actor ownership, no-GC, source-branch authenticity or
complete effect coverage for Java/LLVM. There is no two-file filesystem
transaction: object and effect output may be partially published if a
write fails. Do not consume them as a signed atomic proof receipt.

The CLI rejects source/output aliases and object/effect aliasing, and
caps input source to 1 MiB before parsing. The lower-level effect
exporter remains a non-trust API; only `export_effect_ir_checked` is
an admission gate. The standalone analyzer should still treat JSON as
untrusted and label its reports accordingly.

Verification commands when Cargo is available:

```bash
cargo fmt --all -- --check
cargo test --locked --all-targets
cargo clippy --locked --all-targets -- -D warnings
cargo run --locked --bin oresc -- --compile-with-effects \
  tests/fixtures/forward.ores target/forward.o target/forward.effects.json
```

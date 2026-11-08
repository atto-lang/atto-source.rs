# Java/LLVM canonical scalar-v1 admission by Rust/Cranelift

**Status: NOT_EXECUTED / NOT_CERTIFIED.** No Rust runner or guest execution evidence is claimed by this authored patch. Source provenance: canonical `contracts/atto-scalar-v1` registry and its 15 files on Java and LLVM main (2026-11-08). Expected Git blob IDs:

```text
positive/canonical-add.atto 015f8b0f79aafa796a148d2adeff1574e8d6c491
positive/precedence.atto 50414e5e95c8007f066696f55aa75309555b83e2
positive/parentheses.atto 9f508169c741486d5cec7510ac84fb761f342def
positive/signed-minimum.atto 450ebee951733d3f0b567413e70724155d26ce13
positive/signed-max-minus-one.atto c400f440fde2fbc10514ab7cfd2ffe3d19729ee1
positive/type-first-readonly.atto 75ff4afc98fc65a774611eb7686e893f3d28a6da
positive/typed-const-readonly.atto 0325018c54dd3534ddac76dc29724aa61e979683
positive/inferred-const-readonly.atto 4fe471862d90ba57e135b93728bd44690ff2c894
positive/pure-call.atto e5ab32f7d8f684d655a83a3df989f16c52090c05
positive/type-first-parameter.atto 3d6c16ed04aa86f954461885ec77c8f10b47fb5c
negative/type-first-reassignment.atto b118f53e68565dcb9e6acd58deb8ee47dca67606
negative/typed-const-reassignment.atto 784ebd47a50211e6f00cb732cc74319ed1e58fac
negative/removed-modulo.atto 45d5794870e93074d9b11b90833a849e96aaa4c1
negative/removed-caret.atto 6889c7e83c8d94b906fbfe78723caee9580da6aa
negative/removed-double-caret.atto 7ee99948018e927cce74c9ad0edc1975e4251689
```

The ten positives cover arithmetic precedence, parentheses, full-width signed-i64 boundaries, immutable-by-default local declarations and both `int x` and `x: int` scalar parameter forms. The five negatives cover immutable rebinding and removed operators. The Rust frontend adds **only** name-first typed parameter parsing, preserving the same AST/MIR, duplicate-binding and arity gates. Unsupported or untyped parameters fail closed.

The Rust main compiler does **not** certify the contract's four additional checked-overflow negative cases. The main scalar arithmetic wraps by default; runtime `--checked-i64` traps are not equivalent to a checked compiler diagnostic. The Java/LLVM contract registry does not currently list Rust as a certified route and is not modified here. Rust currently reports `syntax_error` on %, ^, ^^ rather than the normalized `removed-operator` family; this is a known diagnostic-parity gap.

New `tests/atto_scalar_java_llvm_v1.rs` uses real parser, MIR and Cranelift compilation, actual `oresc --strict-aot`/ELF/`cc`/native execution on Linux, stdout/stderr gates, full-width C i64 oracles for both signed extremes, and invalid-object nonpublication/overwrite checks. It also checks AST equivalence, no type inference, duplicate names, and call arity.

Required exact-head verified gates: `cargo fmt --all -- --check`, `cargo clippy --locked --all-targets -- -D warnings`, `cargo test --locked --all-targets`, source blob read-back, stepful native tests, independently verified Java/LLVM actual guest outcomes, and independent review. Zero-step GitHub Actions is runner admission failure, not proof. No claims for actors, 32-bit compressed pointers, heaps, ownership, async, generics or release parity.

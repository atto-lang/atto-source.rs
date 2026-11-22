# Rust/Wasm compiler-owned Atto testing-scalar subset (draft)

This backend independently authors `stdlib/testing_scalar.atto`, rather than
sharing Java, LLVM or JS library source bytes. It uses **0 for pass and 1 for
failure**, with integer equality/inequality, strict greater/less comparisons,
an inclusive inverted-bound-rejecting range check, and status combination.
The range function does no integer arithmetic on endpoints.

Real-guest candidate test:

```sh
cargo build --locked --bin attoc
ATTOC=target/debug/attoc node tests/testing-scalar-atto-guest.mjs
```

The script loads the actual compiler-local .atto source, adds a small
main() invocation for each of 17 deterministic cases, launches the native
attoc compiler, validates emitted guest WebAssembly, instantiates it on
Node and checks the **actual guest main return** and panic status. It
records source/compiler/Wasm SHA-256 values and refuses a missing compiler
or missing guest artifact. These checks are **not executed** merely by
adding the file and no exact-head CI pass is claimed.

This is not an implicit std import implementation. It is source composition
for independently executable test fixtures, pending a compiler-owned
`std/testing_scalar` importer and a fuller TestCase/TestSuite runner.
There is no guest ORES_TEST protocol, property checker, actor test runner,
native AOT or Java/LLVM guest evidence in this candidate. The public
scalar truth table, not implementation source identity, is the parity target.

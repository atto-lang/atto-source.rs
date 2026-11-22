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

The bounded compiler-owned import linker now resolves canonical
`import fnc in_range_int from "std/testing_scalar";` and the
`atto.std/testing_scalar` disambiguation aliases directly from
`include_str!("../stdlib/testing_scalar.atto")`. It cannot call a project
file loader for this module. The single-source `compile()` path admits
this built-in without granting ambient relative-file imports; those still
require `compile_with_imports` and a trusted loader.

The script compiles actual imports for 21 deterministic positive cases,
launches the native attoc compiler, validates emitted guest WebAssembly,
instantiates it on Node and checks the **actual guest main return** and
panic status. Eight additional negative compiler cases reject unsupported
or forged library paths, unexported names, duplicates, collisions and
untrusted relative files, requiring nonzero compiler status and no
emitted Wasm. The script records source/compiler/Wasm SHA-256 values and
refuses a missing compiler or missing guest artifact. These checks are
**not executed** merely by adding the file and no exact-head CI pass
is claimed.

The independent browser-compiler parity driver
`tests/testing-scalar-browser-compiler.mjs` uses the **same compiler-local
21 positive + 8 negative test oracle** as the native driver. It invokes
both the exact-head Rust native `attoc` and the wasm32 compiler binary,
checks byte-identical emitted guest WebAssembly for each positive source,
executes both guests and compares their results, and rejects all negatives
without accepting partial guest artifacts. The main CI workflow invokes
it after building the wasm32 compiler. These tests are authored, not
an executed CI result.

This is a narrow compiler-owned import, **not** general package importing,
a full TestCase/TestSuite runner, or a source graph resolver.
There is no guest ORES_TEST protocol, property checker, actor test runner,
native AOT or Java/LLVM guest evidence in this candidate. The public
scalar truth table, not implementation source identity, is the parity target.

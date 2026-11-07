# Java reference differential: local evidence

The Java reference compiler is private (`ores-truffle-oreslang/oreslang-source.java`), so it is never copied into this org or its CI. The differential runs locally against a built launcher:

```sh
ATTOC=target/release/attoc ATTO_JAVA=<launcher> node tests/conformance.mjs
```

## Run of 2026-11-06

- Reference: oreslang-source.java at `9424b2deeef76fae14f1f8332e8690741c6a5e3e`, GraalVM JDK 25, macOS arm64
- Compiler: atto-source.rs working tree for this commit; wasm32 compiler sha256 `10262395ecb10b42f1526af4bb52367c05b170f4c77e43f5a9119d0d88da5a72`
- Result: `conformance: 99 passed, 0 failed, 88 Java-differential, 94 native-vs-wasm`

What the run checked:

- **46 positive fixtures.** Java exit status equals `main() & 255` from the browser guest, and Java stdout equals guest stdout byte for byte.
- **42 negative fixtures.** Java rejects every fixture marked `// java: rejects`. It accepts each fixture marked `// java: accepts`, which is either a feature the browser compiler has not lowered yet or a recorded divergence.
- **Panic fixtures.** These are excluded from the Java comparison: Java wraps on overflow, while spec 17 and the LLVM backend trap.

During development, the differential corrected two compiler rules to match `Parser.java`:

- implicit newline statement terminators
- `const mut` / `let mut` referent qualifiers

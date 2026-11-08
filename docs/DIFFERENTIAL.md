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
- **Removed operators.** `%`, `^`, and `^^` are rejected with `E_UNSUPPORTED` by the browser compiler; the former `%` fixture programs were rewritten without the removed operator. Java legacy syntax does not override the current AttoLang decision.
- **Eager conditions.** `,` and `|` evaluate both sides in source order; the positive fixture asserts both observable `boom()` calls.
- **Panic fixtures.** These are excluded from the Java comparison: Java wraps on overflow, while spec 17 and the LLVM backend trap.

During development, the differential corrected two compiler rules to match `Parser.java`:

- implicit newline statement terminators
- `const mut` / `let mut` referent qualifiers

## Run of 2026-11-07: classes, actors and enums

- Reference: oreslang-source.java at `9424b2deeef76fae14f1f8332e8690741c6a5e3e` (the same launcher), GraalVM JDK 25, macOS arm64
- Compiler: branch `feat/define-class-actor-enum-java-canonical-20261107`
- Result: `conformance: 171 passed, 3 failed, 137 Java-differential, 148 native-vs-wasm`

The 3 failures are `pass/27-short-circuit`, `pass/47-eager-boolean-precedence` and `pass/48-while-eager-exit`. They fail identically on the unmodified base (`chatgpt/compiler-semantics-eager-no-removed-ops-20261107`: 122 passed, 3 failed). The eager-condition decision in that base differs from the Java build's short-circuit behaviour. This branch does not touch it.

Every class, actor and enum construct was probed against the launcher before it was admitted:

- **19 new positive fixtures** (50–68). Java agrees on exit status and stdout for each, except two cases. `68-enum-values` is `// java: llvm-design`: Java still rejects `define enum`, so the check asserts that rejection. Actor fixtures sometimes hit the Java runtime's shutdown race (`The context is currently executing on another thread`, status 1). Their stdout is then a truncated prefix, so only acceptance and the prefix are compared.
- **30 new or updated negative fixtures** (35, 37, 43–72). Java rejects every one marked `rejects`. It accepts `69-object-identity-comparison` and `72-mail-string-payload`, which are features this compiler has not lowered.
- **Recorded runtime divergence.** Java wraps integer overflow inside actors (it printed `-2` for `2 * 9223372036854775807`). The browser compiler panics, following spec 17. `panic/10-actor-overflow` is excluded from the Java comparison like the other panic fixtures.

Spellings rejected by Java that earlier branches had used are now negative fixtures: `receive(int value): int`, a non-`pub` `static fnc` called from outside, `let n: int = 0` fields and constructing a class without `new`.

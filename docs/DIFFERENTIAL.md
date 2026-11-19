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

## Run of 2026-11-08: floats, strings, match, type-first bindings

- Reference: oreslang-source.java **main `7f35687121`** (2026-11-07), freshly built, GraalVM JDK 25, macOS arm64. The earlier `9424b2d` launcher is three days older; current main changed one admitted form (below).
- Compiler: branch `feat/browser-compiler-values-20261108`, stacked on `feat/define-class-actor-enum-java-canonical-20261107` (#29)
- Result: `conformance: 211 passed, 0 failed, 170 Java-differential, 181 native-vs-wasm` (two consecutive runs)

What changed in the reference and in this compiler:

- **Type-first bindings.** Current Java main accepts `int x = 7;` as an immutable binding (`Parser.looksLikePrimitiveTypeFirstImmutableBinding`: exactly `int`, `bool`, `string`, `String`; reassignment is "cannot reassign const binding"). Spec chapter 21 says the same. The former negative fixture `fail/14` is now `pass/69`. `float f = 1.5;` stays a parse error in Java and here.
- **Eager conditions are a pinned divergence.** `pass/27`, `47` and `48` now carry `// java: diverges` with Java's exact short-circuit output in `// java-stdout:` lines, so the run is green and still fails if either side changes.
- **Actor output under the Java shutdown race.** For `stdout-order: unordered` fixtures the truncated Java output is compared as a sub-multiset of lines (it was compared as a prefix, which failed when two actors printed in the other order).

Every new construct was probed against the launcher before it was admitted, then pinned in fixtures:

- **Floats** (`pass/71–73`, `fail/73`, `74`, `87`, `92`): binary64, Java widening of an int operand, `7.0 / 2 = 3.5`, Java `Double.toString` output (`1.0E7`, `1.0E-4`, `1.234567890125E11`, `NaN`, `Infinity`, `-0.0`), literal lexing (`1_000.25`, `2E-3`, `1.5e+2`; `1.` and `.5` rejected). Divergence: Java keeps an `int` bound to a `float` (prints `3`); here that is `E_TYPE` (`java: accepts`).
- **Strings** (`pass/70`, `74`, `75`, `81`, `82`, `fail/75`, `76`, `88`): content `eq`, `+` of two strings (never `string + int`), `<`/`<=`/`>`/`>=` in `String.compareTo` order including the UTF-16 surrogate case (`"𝄞" < "￼"`), `trim`/`starts_with`/`ends_with`/`contains_literal` edge cases, object printing `P{x=1.5, name=bob, n=2}`. `replace_literal` is accepted by Java but not lowered (`java: accepts`). Java infers `let acc = ""` as the literal type `""` and rejects later assignment, so `pass/82` declares `let acc: string`.
- **Match** (`pass/76–80`, `fail/77–85`, `89–91`): legacy and canonical `over`/`on` forms, `do match` requiring `over`, `else` only in the legacy form, guards evaluated only after the pattern matched, binding patterns, definite return, and Java's "unreachable match arm" / "non-exhaustive match" rejections. `pass/80` (enum value arms) is `java: llvm-design`.

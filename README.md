# atto-source.rs: AttoLang browser compiler

A Rust compiler from `.atto` source to WebAssembly. It is built for `wasm32-unknown-unknown`, so the [atto-playground](https://github.com/atto-lang/atto-playground) compiles and runs programs entirely in the browser. No server sees user source.

It compiles a **subset** of AttoLang. Everything outside the subset is rejected with a diagnostic, and a rejected program never produces an artifact. The compiler does not approximate unsupported features.

## Browser tagged ADT milestone (concrete generic profiles)

The browser Rust-to-Wasm frontend executes **three concrete ADT profiles**:
`Option<int>` (`Some(int)` / `None`), `Result<int,string>`
(`Ok(int)` / `Err(string)`), and `Option<Result<int,string>>`
(`Some(Result<int,string>)` / `None`). The nested profile preserves
the inner Result discriminant, including `Some(Err(...))`, without flattening.
General-purpose `Option<T>` and `Result<T,E>`, other nested ADTs,
mutable ADT payloads, generic classes, and cross-actor ADT transfer
remain unsupported and must not be advertised as implemented.

Constructors are contextual: a typed function return, local binding,
assignment, or argument supplies the exact declared ADT type. For example,
`return Err("failure");` inside a `Result<int,string>` function.
Without such a contextual type, a standalone `Some(...)`, `Ok(...)`,
`Err(...)` or `None` is a type error. Both variants of a tagged ADT
must be covered by unguarded constructor-pattern arms of a
`match ... over`; guards cannot prove exhaustiveness, and duplicate
or wrong-family arms are rejected.

Each value is an immutable 16-byte record in the module's existing
fixed 1 MiB heap, with an i32 discriminant at byte 0 and a 64-bit
payload at byte 8. For the nested profile, the outer Some payload is
an i32 reference to the independently allocated inner Result record;
None has no payload. Both layers are checked before their payloads
are read, and both allocations use the existing quota checks. Allocation uses the same quota-checked object
allocator and panics 3 on exhaustion; values do **not** add capabilities,
imports, reflection, or a different guest ABI. Reference equality is
**not** silently used as value equality: `==` / `eq` and `!=` /
`neq` on ADTs are rejected until structural comparison semantics
are implemented. A stored `Err` or `None` never sets `panic_code`.

The checked-in `tests/fixtures/pass` and `fail` cases are compiled
and executed by both native and wasm32 compiler paths in CI. This work
does not update the pinned public playground compiler or lift the
syntax-preview labels.

### Caller-side trap remains separately gated

The nested ADT profile is groundwork for `trap checked_id(5)`, but does
**not** implement expression-level `trap`. The parser deliberately
rejects that expression with `E_UNSUPPORTED` until the compiler has
guest-proven ordinary-`throw` unwinding. Catching a Wasm engine trap,
checked-overflow panic, `raise`, or cancellation as `None` would be
incorrect. The public playground compiler remains separately pinned
and unchanged.

## Where the syntax comes from

The compiler does not define any syntax of its own:

- **Syntax** follows `docs/grammar.ebnf` and `Parser.java` in `oreslang-source.java`. That includes binding forms, the `const mut` / `let mut` referent qualifier, statement terminators with implicit newline termination, condition `,` / `|`, and arity overloading.
- **`int`** is a signed 64-bit integer with checked arithmetic. Overflow is a panic, never a silent wrap. This matches the LLVM backend's trap and spec chapter 17 (TARGET, decided).
- **Spec** decisions come from the AttoLang spec chapters 01, 16 and 17.

## Admitted subset

| Area | Admitted |
| --- | --- |
| Declarations | `[pub] fnc` / `routine` with `: T` or `-> T`, brace body; parameters `int a`, `int mut a`, `a: int`; arity overloads |
| Entry point | zero-argument `main` returning `int` or `void` |
| Types | `int` (i64), `bool` / `boolean`, `float` / `f64` (IEEE binary64), `string` / `String`, `void` (return only) |
| Bindings | `val` / `const` / `let`, with `name: T`, `T name` or inferred; `const mut` / `let mut`; the immutable type-first form `int x = 7;` for exactly `int`, `bool`, `string` and `String` (Java `looksLikePrimitiveTypeFirstImmutableBinding`) |
| Statements | assignment, `return`, `if … then … elif/elseif/else if … else … fi` and the brace form closed by `fi`, `while … do … done` / `while … { }`, `loop { }`, C-style `for (…; …; i++)`, `break`, `continue`, `block { }`, `match` (below) |
| Expressions | Functional `|>` pipelines (`value |> named_fn |> named_fn(extra)`) lower to left-to-right ordinary calls, with the incoming value as the first argument; only named function stages are supported. `+ - *` on int and float, `/` with at least one float operand (an int operand widens, Java numeric promotion), `+` joining two strings, unary `- + !`, `< <= > >=` on numbers and on strings (Java `String.compareTo` order), `eq neq == != !eq` (strings by content), ternary `? :`, `,` (AND) and `\|` (OR) inside conditions, calls, `And` / `Or` / `Xor` |
| Literals | decimal ints; floats exactly as the Java lexer: `1.5`, `1e3`, `2E-3`, `1_000.25` (digits on both sides of `.`) |
| Strings | members `trim()`, `starts_with(s)`, `ends_with(s)`, `contains_literal(s)` (Java string member table); concatenation allocates on the bounded heap |
| Match | legacy `match [first] x  P -> { } … end` with `else`, and canonical `match x over on P -> { } … end` / `do match x over …` / `over { on P { } }`. Patterns: `_`, int and bool literals, a lower-case binding, `Enum.Variant` (LLVM design spelling); `when` guards on literals and `_`, evaluated only after the pattern matched. Unreachable arms and non-exhaustive matches are `E_MATCH`; a match whose arms all return is a definite return |
| Output | `stdio.print(x)` / `stdio.println(x)` for `int`, `bool`, `float` (Java `Double.toString`: `3.5`, `1.0E7`, `1.0E-4`, `NaN`, `Infinity`, `-0.0`), strings and objects (`P{a=1, b=true}`) |
| Classes | `[pub] define class Name as … end`. Fields of type int, bool, float, string or an enum: `T name: e`, `mut T name = e`, `let T name = e` (assignable in the constructor) and `name: T = e`, `final …`, `val`/`const` (fixed). One `constructor(…)`; instance methods `[pub] [fnc] name(…)`; `[pub] static fnc name(…)`, always called qualified as `Name.f(…)`. Construction is `new Name(…)` |
| Actors | `define actor Name as … end` or `actor Name … end`. State `let T name = e` (mutable) and `val`/`const`/`final` (fixed), one `receive(ActorMail<int\|bool> mail): void` reading `mail.value` and `mail.sequence`, and helper methods called as `self.f(…)`. `val r = spawn Name();` gives a copyable `ActorRef<Name>`, and `r.send(v)` enqueues mail |
| Enums | `define enum Name as A B C end` and `Name.A` values compared with `eq`/`neq`. This spelling comes from the oreslang-source.llvm v1 design contract (`contracts/v1/design/enum_pattern_cases.json`). The Java parser does not admit it yet, and the fixtures record that |

### Class and actor rules (checked against the Java reference)

- Members are private by default. A private field, method or static function is visible only to code inside its own class, which includes other instances of that class.
- Instance methods see a read-only `self`. Only the constructor assigns fields (`E_OWNERSHIP`), and fixed fields reject even that (`E_IMMUTABLE`).
- Objects move. `val b = a`, passing `a` as an argument and `return a` all transfer ownership. Using `a` afterwards is `E_MOVED`, and so is moving an outer object inside a loop. Field reads, method calls on `a` and printing `a` do not move it.
- Actors are isolated. Their bodies cannot reach file-level functions, sibling classes or enums (`E_ISOLATION`). Actor state is private and changes only through mail.
- `spawn` takes no arguments and actors have no constructor (`E_ACTOR`). `receive` is actor-local: it cannot be `pub`, takes exactly one `ActorMail<T>` and returns `void`.

The following are rejected, with the code shown:

- Removed `%`, `^` and `^^` give `E_UNSUPPORTED` with no artifact emitted.
- Condition `,` / `|` evaluate **both** boolean operands left-to-right (eager), including observable side effects.
- `int / int` gives `E_UNSUPPORTED`. Its result type is undecided (spec S-25): Java yields `float` and LLVM rejects it.
- `|>` is functional composition only: receiver stages, actor pipelines and computed callees remain `E_UNSUPPORTED`; stage call arity and types are checked normally. `|` outside a condition gives `E_UNSUPPORTED`. Java treats it as bitwise, and the grammar has no bitwise operators.
- `int / int` stays `E_UNSUPPORTED` (S-25, below); write a float operand.
- `f32`, sized integers, unsupported generic ADT instantiations, arrays and collections give `E_UNSUPPORTED_TYPE`. Admitted profiles are `Option<int>`, `Result<int,string>`, and `Option<Result<int,string>>`.
- `string + int` is `E_TYPE`, as in Java (no implicit conversion). `string.replace_literal`, `split_literal`, `is_identifier` and `is_route_segment` give `E_UNSUPPORTED`; unknown string members are `E_UNBOUND`.
- Match patterns other than the admitted scalar/enum patterns and v1 ADT constructor variants (`Some(x)`, `None`, `Ok(x)`, `Err(x)`) give `E_UNSUPPORTED`. Tuple and type patterns remain unsupported; wrong-family or non-exhaustive ADT patterns fail with typed diagnostics. Negative literal patterns, `1 | 4` alternatives and `name when …` remain `E_SYNTAX`.
- `switch`, value `match<T>`, imports, lambdas, non-v1 generic ADT instantiations, `for … of`, `try`, `defer`, abstract classes, inheritance, generic classes, static fields, enum payloads and values, `spawn` inside actors, and calling actor methods directly all give `E_UNSUPPORTED`.
- Comparing object or actor references, a `self` value (`return self`), ternaries choosing between objects, and class-typed fields give `E_UNSUPPORTED`. Fields need an initializer, and field initializers must be constant expressions.

### Known divergences from the Java reference

These are recorded in fixtures and checked by the differential run:

- Binding the result of a void call. Java accepts it. Here it is `E_TYPE`, because a void call has no value.
- Conditions. `,` and `|` are eager here (PR #4); Java short-circuits. Fixtures pin Java's output with `// java: diverges` and `// java-stdout:` lines.
- An `int` where a `float` is required (`val f: float = 3;`, a float parameter). Java keeps the int and prints `3`; here it is `E_TYPE`.
- `let s = "";` then `s = s + "x"`. Java infers the singleton literal type `""` and rejects the assignment; here the binding has type `string`.
- Integer overflow. Java wraps silently. Here it panics, following spec 17 and the LLVM backend.
- Object references. Java compares them by identity, but here comparison is not admitted yet.
- Object heap. Java has a garbage collector. Here objects come from a fixed 1 MiB bump heap that is never freed, and exhausting it panics with code 3.
- Cross-actor output order. Different actors run concurrently in both runtimes, so lines from different actors may interleave in any order. Within one actor, mail is processed in FIFO order.

## Guest contract `atto.browser.guest.v2`

v2 extends v1. Every v1 guest is still a valid v2 guest.

- **Exports:** `main: () -> i64` (a `void` main returns 0) and `panic_code: mut i32` (1 = checked integer overflow; 2 is reserved, because integer division and remainder are not admitted; 3 = heap exhausted, by objects or string concatenation). `memory` is exported only when strings or classes exist. With classes or string concatenation it is a fixed, non-growable 16-page (1 MiB) memory. A string value is an `i64` `offset << 32 | len` into that memory.
- **Imports:** only the ones used, from module `atto`: `print_i64(i64)`, `print_bool(i32)`, `print_str(i32, i32)`, `print_newline()` and `print_f64(f64)` (the host renders Java `Double.toString`; `tests/java-double.mjs`), plus, when the program spawns actors, `actor_spawn(i32 type) -> i32 ref` and `actor_send(i32 ref, i64 value)`. There are no other capabilities.
- **Actors:** a program with actors also exports `actor_init(i32 type)` and `actor_receive(i32 type, i64 value, i64 sequence)`, and carries a custom section `atto.actors` listing actor type names in type order. The host runs each spawned actor in its own instance of the same module (one dedicated Web Worker per actor in the playground). It calls `actor_init` once and then `actor_receive` once per message, in FIFO order with sequence numbers from 0. Actor state lives in that instance's globals, so actors share no memory. Bool mail is passed as 0 or 1.
- **Determinism:** output is a pure function of the source. The native and wasm32 hosts emit byte-identical modules.

## Compiler ABI v2 (wasm32 build)

The compiler has no imports and memory capped at 64 MiB. It exports:

- `abi_version()`, which returns 2
- `memory`, `input_ptr()` and `input_capacity()` (64 KiB)
- `compile(len)`, which returns 0 on success and 1 on a diagnostic
- `result_ptr()` and `result_length()`, which are 0 on error
- `error_ptr()`, a NUL-terminated `E_CODE: message`
- `error_line_number()` and `error_column_number()`

## Resource limits

Every limit fails closed with a diagnostic; none of them can trap the compiler.

| Limit | Value | Code |
|---|---|---|
| Source size | 64 KiB of UTF-8 | `E_SOURCE_LIMIT` |
| Tokens | 32,768 | `E_SOURCE_LIMIT` |
| Recursive nesting (parens, blocks, unary operators, ternaries, calls) | 128 levels | `E_NESTING` |
| Syntax-tree height, counting each link of an operator chain, each `.member` / `.method()` link and each `elif` arm | 256 | `E_NESTING` |
| Top-level declarations / parameters / locals per function | 1,024 / 32 / 1,024 | `E_SOURCE_LIMIT` |
| Members per class or actor; variants per enum | 256 | `E_SOURCE_LIMIT` |
| One string literal / all string data | 4 KiB / 32 KiB | `E_SOURCE_LIMIT` |
| Generated module | 1 MiB | `E_OUTPUT_LIMIT` |

The tree-height budget exists because `a + b + c + ...` is parsed in a loop
but produces a left-leaning tree that the checker and emitter walk
recursively; without it a 30 KB source exhausted the stack. The checker keeps
its recursive path (operators, member and method chains) in small
`#[inline(never)]` frames. At 256 levels every worst case, member and method
chains included, fits in 1.5 MiB of stack in a debug build (measured with
`examples/stackprobe.rs`), so `compile()` is safe on a default 2 MiB thread.

## Build and test

```sh
rustup toolchain install 1.85.1 --profile minimal --target wasm32-unknown-unknown
cargo test --locked
cargo build --locked --release --bin attoc
cargo rustc --locked --release --lib --target wasm32-unknown-unknown --crate-type cdylib -- -C link-arg=--max-memory=67108864
ATTOC=target/release/attoc node tests/conformance.mjs
# Optional Java reference differential (local only; the reference is private):
ATTO_JAVA=/path/to/oreslang-java-launcher ATTOC=target/release/attoc node tests/conformance.mjs
```

The fixture corpus is in `tests/fixtures/`:

- `pass/`: expected `main()` result and stdout
- `pass/`: a `// java: llvm-design` claim marks a spelling taken from the LLVM design contracts that Java must still reject. `// stdout-order: unordered` marks output from several concurrent actors, compared as a multiset of lines.
- `fail/`: expected diagnostic code, plus a `// java: rejects|accepts` claim
- `panic/`: expected panic code

`tests/conformance.mjs` checks the following:

- It compiles every fixture with the wasm32 compiler and executes every guest.
- It compiles 200 generated programs.
- It feeds the compiler 2,000 random inputs.
- It checks the nesting, tree-height and source limits, at and just over each budget.
- It feeds the compiler adversarial shapes (15,000-link chains, 30,000 unary operators, unclosed nesting) and requires a diagnostic, never a trap.
- It mutates every positive fixture at the token level (60 mutants each; set `ATTO_FUZZ=<n>` for more) and requires every accepted mutant to be valid Wasm and every rejection to carry a code and position.
- It checks recovery after an error.
- With `ATTOC` set, it requires byte-identical native output.
- With `ATTO_JAVA` set, it requires the Java reference to agree on exit status, stdout and acceptance for each fixture.

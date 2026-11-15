# atto-source.rs: AttoLang browser compiler

A Rust compiler from `.atto` source to WebAssembly. It is built for `wasm32-unknown-unknown`, so the [atto-playground](https://github.com/atto-lang/atto-playground) compiles and runs programs entirely in the browser. No server sees user source.

## Namespace and out-of-line receiver syntax (parse-only)

This frontend now has a bounded, parse-only grammar for the proposed use namespace
header, multiple declare actor statements in one file, and typed mtd receivers
such as (mut self: Foo1) and (self: Foo2), as well as the nominal
spelling (Foo1 self), unnamed (Foo1), and (Foo1 _) receivers.
The parser finds target declarations anywhere at file scope, while the
namespace header must still occur first. Explicit module imports of the form
import module math from "./math.atto" can follow that header. The parser rejects duplicate actor
names, same-owner/name/arity receiver signatures, unknown receiver owners,
misplaced namespace headers, malformed signatures and excessive body nesting.

These are target declarations only. A new conservative semantic preflight resolves
receiver headers by namespace, nominal owner, name and arity (and checks public
visibility for external header lookups). It also rejects direct self field access,
unqualified handle_state, writes in critical-read, critical-write on immutable
receivers, escape of a bare state callback binder, suspension inside a critical
callback, and direct synchronous method calls on obvious local spawn bindings.
Unknown state field methods fail closed rather than being guessed pure.

## Typed receiver signature IR (not executable)

### Production AST linking (still deliberately no actor dispatch)

The target declaration parser now admits concrete `define actor Foo as ... end`
bodies alongside `declare actor Foo;`, and delegates concrete body parsing
to the existing production actor parser. The typed signature stage builds a
`Program` with ordinary `ActorDecl` nodes and a distinct, non-executable
`ActorInterface`. The production checker validates forward-definition
ownership and rejects duplicate concrete definitions, colliding state fields,
and duplicate or conflicting in-actor/out-of-line signatures. Forward-only
actors remain unlinked. Even when a concrete body parses, it is not yet
fully typechecked for ownership/effects and does not become spawnable in
this target profile; the browser still returns `E_UNSUPPORTED` with no Wasm.

The signature layer in src/target_typed.rs now resolves all nominal actor
declarations before typing the receiver parameter and return annotations.
Known scalar types and ActorRef<Actor> are resolved. Unknown names, void
parameters and bare actor values fail closed. ActorRef is a capability,
not permission to synchronously invoke receiver methods. The IR preserves
namespace, actor index, method name, arity, visibility and mutable receiver
semantics. A forward declaration alone is never spawnable (E_UNLINKED).

This signature graph is not the executable compiler AST and does not
typecheck method bodies, link definitions, or prove effect/borrow safety.
Compile constructs it before returning E_UNSUPPORTED; no guest actor Wasm
is emitted for these declarations.

This is NOT a full typed borrow checker: opaque field types, alias analysis,
effect inference, actor body linkage, cross-file symbols and actor runtime dispatch
remain unimplemented. Positive syntax checks do NOT generate guest Wasm; codegen
continues to return E_UNSUPPORTED for this profile. The public compile entrypoint still returns E_UNSUPPORTED for
this syntax, with no generated WebAssembly.

It compiles a **subset** of AttoLang. Everything outside the subset is rejected with a diagnostic, and a rejected program never produces an artifact. The compiler does not approximate unsupported features.

## First executable synchronous generator profile (bounded guest Wasm)

The compiler now emits **real guest WebAssembly** for a deliberately narrow
`fnc *gen name([int limit]): Generator<int>` profile: zero or one
immutable `int` parameter, finite source-ordered `yield` statements
containing nonnegative signed-i64-compatible integer literals, **one**
strictly checked loop form `for (let i = N; i <= limit; i++) do yield i; done`,
and at most one `yield* child()` or `yield* child(limit)` per
function. Delegation targets are statically checked and acyclic; their
arguments may be a signed-i64-compatible nonnegative literal or their
parent's immutable parameter. Computed delegate arguments remain gated.
Generator creation is lazy: it allocates a uniform 184-byte arena-owned
continuation frame (immutable i64 parameter, legacy counter, one child handle,
and up to 16 isolated i64 live-local slots). Every generator uses the same
frame size to make recycling safe across generator types. Only a consumer
pull advances
its resume PC and yields one item. The typed `for value in values() do ...
done` loop owns a **fresh** iterator and automatically calls the guest
close operation on completion, `break`, and early function `return`.
Closed frames are recycled with incremented generations, detecting stale
handles and forbidding a double insertion into the free list. Before
entering the free list, close transitions through CLOSING, detaches and closes
an active child, then wipes the last yielded value, captured scalar parameter,
counter and all 16 local slots. Header generation survives closure solely
for stale-handle fencing. The checked handle verifier also checks the actual
allocated arena high-water mark, not only the fixed Wasm memory limit.

The typed integer continuation CFG now performs fixed-point definite
initialization (intersection across all predecessor paths) and may-liveness
(union across every successor, including loop backedges). After each yield,
guest Wasm clears integer slots proven dead on every future path while
retaining loop-carried values. Object/borrowed values are still rejected:
this scalar-only analysis does **not** establish general borrow safety or
exception/cancellation-safe unwinding.

Delegation creates a child frame only on the first pull through its
`yield*` state, advances that child by at most one element per outer pull,
and transparently resumes outer statements after child DONE. Early outer
close recursively closes its active child before recycling the outer frame.
Delegation cycles reject with `E_GENERATOR_CYCLE` and chains deeper than
64 nested generators reject before guest emission.

The runtime's `new`, `checked_ptr`, `next`, and `close` are compiled
into the guest module; they do **not** use JavaScript generators, an
interpreter, or extra host imports/exports. Frames reside in the fixed
1 MiB guest-instance heap, not in a cross-actor global heap. Invalid
generator state/handle access panics with `panic_code=4`; heap exhaustion
continues to use `panic_code=3`.

**Now additionally admitted:** bounded typed control-flow generators
with up to 16 persistent mutable/immutable `int` locals, one immutable
`int` parameter, pure checked addition/subtraction/multiplication,
integer comparisons, boolean conditions, `if`, `while`, and
`break`/`continue`. Their CFG state advances through each expression and
branch, with locals restored on every `next()`. Declarations must be
top-level before control flow, cannot shadow other bindings, and must be
definitely initialized. Each pull has a 65,536-instruction dispatch budget;
a non-yielding loop traps deterministically with `panic_code=5`.

**Not yet admitted:** multiple generator parameters, object or borrowed
live locals, general calls/effects across suspension, nested-scope
declarations, nested generator-owned iterator loops, more than one
`yield*` per generator, exception-throwing or otherwise effectful
for-in bodies, async suspension, cancellation unwinding, and cross-backend
generator parity. Unsupported programs fail closed; there is no eager
substitution. Checked arithmetic traps terminate the guest instance rather
than supporting resumable unwinding. This is **not** the full AttoLang
generator language. The publicly pinned playground compiler remains
independently gated and must not relabel `yield*` previews as runnable.

The exact-blob alternate-runner conformance tests in
`tests/generator-execution.mjs` run real guest Wasm, verify byte-identical
native and wasm32 compiler output, 12,000-iteration frame reuse, early return and break cleanup, empty
generators, lazy nested `yield*`, cycle rejection, exact closed-frame
zeroization/generation checks, and unsupported construct rejection.

## Browser tagged ADT milestone (concrete generic profiles)

The browser Rust-to-Wasm frontend executes **three concrete ADT profiles**:
`Option<int>` (`Some(int)` / `None`), `Result<int,string>`
(`Ok(int)` / `Err(string)`), and `Option<Result<int,string>>`
(`Some(Result<int,string>)` / `None`). The nested profile preserves
the inner Result discriminant, including `Some(Err(...))`, without flattening.
General-purpose `Option<T>` and `Result<T,E>`, other nested ADTs,
mutable ADT payloads, generic classes, and cross-actor ADT transfer
remain unsupported and must not be advertised as implemented.

Postfix `?` now propagates the first executable tagged ADT profiles:
`Result<int,string>` (extract `Ok(int)`, return the original `Err`)
and `Option<int>` (extract `Some(int)`, return the original `None`).
The enclosing function must return the exact same ADT profile; a function
returning `int` or a different ADT cannot use it. The source is evaluated
once, the failure path reuses its original tagged allocation, and the
compiler does not convert `Err` or `None` into a panic. Ternary `? :`
remains a distinct expression operator. This does not yet support
generic ADTs, conversions, or `trap` exception capture: no throwable
function effects are executable in this browser subset, so caller-side
`trap checked_id(5)` still rejects instead of being simulated.

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

### First executable caller-side `trap expr` and ordinary guest throws

The Rust/Wasm browser subset admits synchronous `trap expr` for the two
**concrete** operand profiles `int` and `Result<int,string>`. The result
is `Option<int>` or `Option<Result<int,string>>` respectively, without
flattening `Some(Err(...))`. The operand is evaluated once, before the
Some allocation. The only ordinary throwable currently admitted is a
source-level `throw Error(message)` statement in a **top-level free
function**, where `message: string` can be computed dynamically.
A raw `throw "text"`, forged `Error` constructor, or non-string Error
payload fails static checking.

The compiler emits one **private WebAssembly exception tag** whose sole
payload is the guest Error's i64 string view. It neither imports nor
exports that tag. The `trap` catch handles only that tag and materializes
`None`; throws automatically unwind through ordinary Wasm function calls.
Host exceptions, unknown tags, checked-arithmetic panic, heap exhaustion,
forced cancellation, and fatal engine traps **bypass** this catch.

This is deliberately **not full language exception parity**: arbitrary
Error subclasses, first-class Error values, `try/catch`, `recover`,
`defer` cleanup, async/resumable traps, actor/supervisor exception routing,
`Option<void>`, `Option<Option<T>>`, and general `Option<T>` remain gated
by explicit unsupported diagnostics. No existing guest imports/exports or
`panic_code` meanings change; modules without throws/traps emit no EH
section. The generated guest needs an engine with WebAssembly exception
handling enabled.

The checked-in `tests/fixtures/pass/107-playground-trap-expression.atto`
compiles the actual public playground preview body under the candidate
compiler and executes it in Wasm CI. The **public playground binary** is
pinned separately and has not been replaced or relabeled by this change.

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
| Generators (restricted) | One optional immutable `int` parameter, up to 16 persistent `int` locals and pure typed CFG `if`/`while`/`break`/`continue`, checked scalar arithmetic and computed `yield`; plus earlier canonical `CountTo`/limited lazy `yield*` profile. Owned fresh-handle `for ... in` and bounded guest-Wasm 65,536-step pull budgets. Borrowed objects, arbitrary effects, nested-scope binds and advanced cancellation remain gated. |
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

- **Exports:** `main: () -> i64` (a `void` main returns 0) and `panic_code: mut i32` (1 = checked integer overflow; 2 reserved; 3 = heap exhausted). `memory` is exported when strings, classes, **or actors** exist. Scalar-only actor modules export one bounded 64 KiB Wasm32 memory per instance; allocating classes or concatenating strings uses the existing fixed 16-page (1 MiB) memory. This is guest linear memory, not a suspended Wasm execution stack. A string value is an `i64` `offset << 32 | len` into that memory.
- **Imports:** only the ones used, from module `atto`: `print_i64(i64)`, `print_bool(i32)`, `print_str(i32, i32)`, `print_newline()` and `print_f64(f64)` (the host renders Java `Double.toString`; `tests/java-double.mjs`), plus, when the program spawns actors, `actor_spawn(i32 type) -> i32 ref` and `actor_send(i32 ref, i64 value)`. There are no other capabilities.
- **Actors:** a program with actors also exports `actor_init(i32 type)` and `actor_receive(i32 type, i64 value, i64 sequence)`, and carries a custom section `atto.actors` listing actor type names in type order. The host runs each spawned actor in its own instance of the same compiled module. Worker topology is a separate host concern; an upgraded tenant host can schedule multiple actor instances within one Worker. It calls `actor_init` once and then `actor_receive` once per message, in FIFO order with sequence numbers from 0. Actor state lives in that instance's globals, so actors share no memory. Bool mail is passed as 0 or 1.
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

## Browser tenant worker artifact (experimental)

`atto_compiler::browser_worker::compile_browser_tenant(source)` compiles the existing AttoLang subset to real Wasm and reads the compiler-produced `atto.actors` custom section into a bounded actor-name list. It exposes `atto.browser.guest.v2` plus the `atto.browser.tenants.v1` worker envelope contract for bundlers targeting [atto-browser](https://github.com/atto-lang/atto-browser). The original `compile` ABI remains unchanged.

The browser host runs one Web Worker per tenant and may instantiate several actor-specific Wasm instances within it. This helper does not bundle workers or provide a security boundary; actual Wasm admission, capability grants, worker termination, and runtime conformance remain responsibilities of the host. Run `cargo test` for the manifest tests; no Rust execution proof is claimed until CI verifies the branch.

### Cross-tenant browser route v1 ABI

The browser host can deliver a tenant-authorized signed `i64` message to a live compiled actor. The **external** route payload is 8 little-endian bytes, decoded in the destination Web Worker and passed to the compiler-emitted `actor_receive(i32 type, i64 value, i64 sequence)` export. The broker, not the Wasm guest, controls target actor ID, generation, and capability validation. The Rust contract test `tests/browser_route_wire.rs` verifies the actual emitted Wasm receiver signature and signed wire representation.

This is **not yet compiler lowering for cross-tenant AttoLang `actor.send` expressions**: routing is initiated via the application-controlled `BrowserTenantClient.grant_route/send_route` API in `atto-browser`. `rt take` and `rt copy` in the host are `ArrayBuffer` transfer/copy modes and do not yet extend the compiler's ownership analysis across Web Worker boundaries.

### Native AttoLang source-level remote actor sending (experimental browser profile)

A narrow, typed `remote_actor(ActorType, slot)` expression yields an `ActorRef<ActorType>` without embedding any tenant token or browser route capability into Wasm. The actor type must be declared with a `receive(ActorMail<int>)` handler, and the slot must be a literal integer 1–16. Normal `.send(int)` compiles through the existing `atto.actor_send(i32 ref, i64 value)` guest import. The browser host interprets negative actor references as opaque, supervisor-bound remote slots; positive actor references remain local actors.

`tests/fixtures/browser/remote-actor-source-send.atto` defines a real `Sender` actor whose receive method calls `remote_actor(Counter, 1).send(mail.value)`. The application must explicitly grant and bind slot 1 using `BrowserTenantClient.grant_route()` and `bind_route()` before invoking the sender guest. `tests/fixtures/browser/remote-actor-receiver.atto` supplies an isolated receiver.

This v1 integration **does not yet provide per-send `Result<T,E>`, guest suspension, guaranteed delivery after crash, dynamic route acquisition, or compiler-checked cross-worker `rt take`/`rt copy`**. The supervisor returns a batch-level completion acknowledgement only after the destination handler finishes each send; failures restart the source tenant. This is an intentional, limited bridge, not full remote actor semantics.

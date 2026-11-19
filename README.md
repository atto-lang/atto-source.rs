# atto-source.rs: AttoLang browser compiler

## Explicit deep-copy operator (bounded generated-class AOT milestone)

The production Rust/Wasm frontend recognizes contiguous `copy!!` as a
separate ownership operator. The separated spelling `copy! !expr` remains
a shallow scalar copy of logical negation. The source operand is evaluated
**exactly once** and the sender's original local remains valid.

- Scalars (`int`, `bool`, `float`, immutable `string`, nominal `enum`) are
  copied by value without a heap allocation.
- **Ordinary flat classes** whose full stored field inventory consists only
  of these identity-free values now receive a compiler-generated, private
  AOT function. It bump-allocates a fresh class object, then performs typed
  load/store of every field. It does not invoke the class constructor or
  re-run field initializers. Each invocation allocates a separate root.
- `copy!` on classes is **not** enabled by this milestone; it still requires
  its own separately verified shallow Copyable witness.
- Classes with nested class/actor/collection/resource references, state
  containers, and erased/unproved types fail closed with
  `E_COPY_DEEP_UNSUPPORTED`. A class with an explicit `copy_deep` method is
  rejected for this operator with `E_COPY_OVERRIDE` until verified hook
  chaining is implemented. Exclusive mutable loans reject with `E_BORROW`.

This is **not yet a general graph copier**. The current Wasm object format
has no general dynamic-subtype tag, and the baseline browser frontend does
not lower mutable `Array<int>` or inherited class layouts. Graph cycles,
nested reference identity, memoized copy contexts, native AOT targets,
`Object.copy_deep()` methods, actor mailmen, and cross-actor Sendable/
Serializable provenance all remain separate implementation milestones.

Pass/fail `.atto` fixtures are consumed by the native Rust compiler test
harness and the existing guest Wasm execution harness. The newly added
`tests/copy_doublebang.rs` checks actual compiler-produced Wasm bytes;
it does not alone prove runtime isolation. **Only a test run whose job steps
executed on the exact branch commit can be reported as PASS.** Source-org
GitHub Actions currently failed with zero recorded steps; those jobs are
`NOT_EXECUTED`, not compiler or guest failures.

The compiler now imports the canonical closed-world copy-plan preflight
directly from the borrow-checker. `src/copy_graph_v1.rs` has **identical Git
blob SHA** `3008a139cfba41131839de3567a263163152cf00` to the
Rustfmt-hardened borrow-checker model. The compiler constructs copy
descriptors from actual resolved class field types and authenticates the
concrete flat class before **both** emitting generated copy code and lowering
a `copy!!` call. The source CI hashes the vendored module to detect drift.
This cannot authenticate dynamic mutable Array graphs or prove actor Sendable.

A full **728-blob source-tree** mirror passed funded alternate-org CI:
Rust 1.85.1 full Cargo tests, 492 AttoLang conformance assertions, 444
native-versus-Wasm compiler comparisons, and native/Wasm compiler builds
([CI run #44137504046](https://github.com/gha-indie-worker/atto-ci/actions/runs/44137504046)).
Whole-project formatting and strict Clippy still have pre-existing drift;
no green whole-project lint claim is made. The bounded actor graph copying
model has its own 196-test Rust receipt, but does not lower `Array<T>`
or actor messages in this compiler.

Candidate normative contract: atto-lang/atto-spec PR #39. Canonical
type-graph admission and the nonproduction GraphCopyContext model:
ores-truffle-oreslang/atto-borrow-checker.rs PR #32.
Shared conformance candidates: atto-lang/atto-platform-conformance PR #64.

A Rust compiler from `.atto` source to WebAssembly. It is built for `wasm32-unknown-unknown`, so the [atto-playground](https://github.com/atto-lang/atto-playground) compiles and runs programs entirely in the browser. No server sees user source.

## Canonical AttoLang naming and specification

This is the canonical [atto-source.rs](https://github.com/atto-lang/atto-source.rs) home. Use `.atto` source, `atto-*` names, and `https://github.com/atto-lang/atto-source.rs.git` for Git remotes. Historical naming is not part of current usage documentation.

- **Normative contract:** [atto-spec](https://github.com/atto-lang/atto-spec), including the versioned ownership, borrowing, equality, lifetime, effects and module-constant contracts. New implementation work must use these specs, not old backend examples.
- **Guest parity:** [atto-platform-conformance](https://github.com/atto-lang/atto-platform-conformance) and [atto-compilers-parity-testing](https://github.com/atto-lang/atto-compilers-parity-testing). Pin the exact source/tree and toolchain; distinguish passed, failed, unsupported and not-executed outcomes.
- **Shared and target requirements:** [atto-common-source](https://github.com/atto-lang/atto-common-source) and [atto-target-profiles](https://github.com/atto-lang/atto-target-profiles). Parser-only acceptance and host-only tests do not prove guest execution.

## Identity-free value class admission (NOT EXECUTABLE)

The proposed `define value class` and visibility variants are recognized before product/nominal lowering and fail with `E_VALUE_UNSUPPORTED`, without guest Wasm output. The current compiler's class-backed struct/record profile is **not** an identity-free value representation. This is a safety gate only, not value construction, recursive equality, copying, inline layout or AOT performance support. See [atto-spec #60](https://github.com/atto-lang/atto-spec/pull/60) and [conformance #57](https://github.com/atto-lang/atto-platform-conformance/pull/57).

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

- **Syntax** follows the current AttoLang language specification and checked-in parser grammar. That includes binding forms, the `const mut` / `let mut` referent qualifier, statement terminators with implicit newline termination, condition `,` / `|`, and arity overloading.
- **`int`** is a signed 64-bit integer with checked arithmetic. Overflow is a panic, never a silent wrap. This matches the LLVM backend's trap and spec chapter 17 (TARGET, decided).
- **Spec:** normative language decisions are maintained in the current [atto-spec](https://github.com/atto-lang/atto-spec), including versioned feature contracts. The table here reports only this compiler profile, not a frozen cross-backend specification.

## Admitted subset

| Area | Admitted |
| --- | --- |
| Declarations | `[pub] fnc` / `routine` with `: T` or `-> T`; C brace bodies (`: T {}`, `: T -> {}`, `-> T {}`) or comma/`end` bodies (`-> T … end`, `-> T :: … end`); parameters `int a`, `int mut a`, `a: int`; arity overloads |
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
| Enums | `define enum Name as A B C end` and `Name.A` values compared with `eq`/`neq`. This spelling comes from an earlier LLVM design contract (`contracts/v1/design/enum_pattern_cases.json`). The Java parser does not admit it yet, and the fixtures record that |

### Confirmed callable body syntax (accepted by atto-spec #19)

The five declaration-body forms below apply to `fnc`, direct-call-only `routine`,
and inline instance methods (bare `method_name`, legacy `fnc method_name`
or `method method_name`). These are alternatives, not simultaneous overloads:

```atto
fnc five(): int { return 5; }
fnc five(): int -> { return 5; }
fnc five() -> int { return 5; }
fnc five() -> int let int x = 3, x + 2 end
fnc five() -> int :: let int x = 3, x + 2 end
```

C bodies retain explicit returns; outer Erlang-like bodies use commas between
statements and return their final expression, subject to the normal type,
ownership and control-flow checks. Neither style changes the other style's
terminators. A `->` after `: T` requires `{`. The `=>` fat arrow remains
for *callable type* declarations, not these function body headers.

Out-of-line `mtd` supports named receiver headers `(self: T)`,
`(T self)`, implicit `(T)`, `(T _)`, and explicit `(mut self: T)`
in the **target preflight**. The formatter contract prefers
`(self: T)` for named receivers, `(T)` for implicit receivers and
bare method names for inline instance methods.

For a file containing only local classes and functions, **immutable class
`mtd` receivers are executable**. The lowering reuses the full production
class parser, typechecker, and guest Wasm emitter; each method has the same
ABI and visibility as an equivalent inline method. `(mut self: T)` is
recognized but **fails with `E_UNSUPPORTED`** until mutable-self ownership
rules are implemented. Actor/namespace/import-linked receivers remain
syntax-only and must not acquire unverified synchronous dispatch.
Actor sealed-state restrictions remain unchanged. Executable support must
be backed by actual Rust/Wasm guest tests rather than syntax acceptance.

### Class and actor rules (checked against the Java reference)

- Members are private by default. A private field, method or static function is visible only to code inside its own class, which includes other instances of that class.
- Instance methods see a read-only `self`. Only the constructor assigns fields (`E_OWNERSHIP`), and fixed fields reject even that (`E_IMMUTABLE`).
- Objects move. `val b = a`, passing `a` as an argument and `return a` all transfer ownership. Using `a` afterwards is `E_MOVED`, and so is moving an outer object inside a loop. Field reads, method calls on `a` and printing `a` do not move it.
- Explicit prefix `borrow! owner` and `borrow! mut owner` admit lexical class loans with simultaneous immutable readers or one exclusive mutable writer; `copy! value` and `share! value` accept only verified immutable scalars; `take! owner` invalidates named locals. Parenthesized operands (e.g. `borrow!(owner)`) remain accepted for compatibility, but the formatter's canonical syntax omits grouping around a bare name. Owned class loans cannot escape by return, argument, assignment or take. Moves from loop-external locals are checked in every repeated position, including `while` conditions and `for` condition/update expressions; a `for` initializer executes once and may transfer ownership. This is a bounded native/browser Wasm subset, **not** generalized inter-actor sharing, mutable scalar loans, Arc, suspension, or callable lifetime inference.
- Actors are isolated. Their bodies cannot reach file-level functions, sibling classes or enums (`E_ISOLATION`). Actor state is private and changes only through mail.
- `spawn` takes no arguments and actors have no constructor (`E_ACTOR`). `receive` is actor-local: it cannot be `pub`, takes exactly one `ActorMail<T>` and returns `void`.

The following are rejected, with the code shown:

- Removed `%`, `^` and `^^` give `E_UNSUPPORTED` with no artifact emitted.
- Condition `,` / `|` evaluate **both** boolean operands left-to-right (eager), including observable side effects.
- `int / int` gives `E_UNSUPPORTED` here. Consult atto-spec for the current language contract, without assuming other compiler behavior.
- `|>` is functional composition only: receiver stages, actor pipelines and computed callees remain `E_UNSUPPORTED`; stage call arity and types are checked normally. `|` outside a condition gives `E_UNSUPPORTED`. Other backends must follow the current bitwise and stream operator specification.
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
- Integer overflow. This subset panics on overflow; verify other backend behavior through executed parity tests.
- Object references. Java compares them by identity, but here comparison is not admitted yet.
- Object heap. Java has a garbage collector. Here objects come from a fixed 1 MiB bump heap that is never freed, and exhausting it panics with code 3.
- Cross-actor output order. Different actors run concurrently in both runtimes, so lines from different actors may interleave in any order. Within one actor, mail is processed in FIFO order.

## Guest contract `atto.browser.guest.v2`

v2 extends v1. Every v1 guest is still a valid v2 guest.

- **Exports:** `main: () -> i64` (a `void` main returns 0) and `panic_code: mut i32` (1 = checked integer overflow; 2 reserved; 3 = heap exhausted; 4 = invalid generator handle/state; 5 = generator dispatch budget; 6 = logical stack exhausted: a call on a recursive cycle would exceed 1024 live recursive frames per `main` or receive invocation, the same quota as the Java backend. Only calls on a recursive cycle of the call graph are counted, so modules without recursion are byte-identical to before). `memory` is exported when strings, classes, **or actors** exist. Scalar-only actor modules export one bounded 64 KiB Wasm32 memory per instance; allocating classes or concatenating strings uses the existing fixed 16-page (1 MiB) memory. This is guest linear memory, not a suspended Wasm execution stack. A string value is an `i64` `offset << 32 | len` into that memory.
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
ATTO_JAVA=/absolute/path/to/verified/java-compiler-launcher ATTOC=target/release/attoc node tests/conformance.mjs
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

## Repository ownership and contribution routing

**Belongs in this repository:** The Rust-authored compiler frontend/backend, its documented .atto subset, source-to-Wasm generation, native Rust tests, and wasm32-unknown-unknown compiler artifact for the playground.

**Belongs elsewhere:** The JavaScript transpiler (atto-source.js), the host-independent Wasm backend/runtime contract (atto-source.wasm), or the website/deployment (atto-playground / atto-lang.github.io).

**Safety and verification boundary:** A compiler compiled to browser Wasm is not proof that its emitted guests use Wasmtime in a browser. Browser guests use WebAssembly APIs/Workers; native Wasmtime claims require separate native execution receipts.

Keep canonical AttoLang (`.atto`) semantics consistent with the language specification; migration-era behavior is not a feature-readiness claim.

Changes asserting compiler/runtime support must identify the exact source head, target, toolchain, executed positive and negative guest tests, and failure/unsupported status; an empty CI step list is **not executed**. Documentation records responsibility, **not** a claim of feature completion.

## Draft I/O direction shorthand (not general channel or stream execution)

The browser subset currently has a **draft**, unverified real-lowering profile
for standalone stdout writes only:

```atto
stdio.out << "effectful output";
writech stdio.out, "effectful output";
```

Both statements reuse the same typed `stdio.print` guest Wasm emission and
host import. This profile does **not** add any new guest ABI or a general
`Writable<T>`/channel runtime. It also does not append a newline.
Other channel/stream read or write targets and all `select` I/O arms continue
to fail closed with `E_UNSUPPORTED`. Runtime `const` read bindings fail with
`E_CONST_EFFECT`; use `val`/ `let` for effectful reads once supported.

Tests: `cargo test --locked --test io_shorthand`, then
`cargo build --locked --release --bin attoc` and
`ATTOC=target/release/attoc node tests/io_shorthand_guest.mjs`.
These are intended checks, not an assertion of passing CI. As of the draft
commit, source/private-carrier Actions have not reported executed test steps;
treat guest conformance as **NOT_EXECUTED** until a real run succeeds.
See [language spec](https://github.com/atto-lang/atto-spec/pull/32) and
[remaining implementation tracker](https://github.com/atto-lang/atto-source.rs/issues/180).

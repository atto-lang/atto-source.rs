# AttoLang lambda return-type syntax (parser profile v1)

The preferred braceless form is `|parameters| : ReturnType -> body`.
The annotation is optional: return types should normally be inferred from
expressions and contextual callable signatures (when the relevant backend
supports closure typing). Do **not** write `=>` in new formatted code.

```atto
val values = rx.Observable.from_iter([1, 2, 3, 4, 5]);
val small_doubled = values
    .filter(|n| : bool -> return n < 4;)
    .map(|n| : int -> return n * 2;)
    .take(2);
```

Accepted forms (syntax):

- Inferred statement: `|n| -> return n * 2;`
- Explicit statement: `|n| : int -> return n * 2;`
- Inferred expression: `|n| -> n * 2`
- Explicit expression: `|n| : int -> n * 2`
- Braced: `|n| -> int { return n * 2; }`
- Legacy typed braceless: `|n| => int -> return n * 2;` (accepted for compatibility, not preferred)

The semicolon is part of a braceless `return` statement. It is required
inside a call argument before the closing `)`. For a same-line standalone
binding such as `val cb = (|n| -> return n;);`, the inner `return;` and
outer `val;` have distinct statement terminators. Expression bodies do not need
an inner semicolon. Multiple statements require `{ ... }`.

## Grammar and disambiguation

```ebnf
lambda             = "|", [parameters], "|", [":", type], "->", lambda_body
                   | "|", [parameters], "|", "=>", type, "->", lambda_body ;
parameters         = identifier, {",", identifier} ;
lambda_body        = expression | "return", expression, ";" | block ;
block              = "{", {statement}, "}" ;
```

The `:` annotation belongs to the parameter-pipe pair. The following `->`
marks the body boundary. `|` remains boolean OR in conditional-expression
contexts; a `|` in primary-expression position opens a lambda parameter list.
Qualified and nested generic return-type syntax is preserved, including
keyword-like module path segments such as `my.types.Response`. Speculative
recognition of the legacy braced form restores parser state and propagates
nesting/security-limit errors instead of silently retrying an expression.
Names and type
resolution are separate from parsing. Repeated parameter names fail closed.

## Implementation status

`atto-source.rs` recognizes these lambda expressions as AST nodes, including
those nested inside method-call arguments. Parsing these forms **does not** imply general callable value support. The
browser compiler now admits tightly bounded direct lambdas and one owned
escaping `fnc(int)->int` factory profile, described below. General callable
values, typed indirect Wasm dispatch, mutable captured state, Rx Observable
methods and cross-actor transports remain unsupported.
It reports `E_UNSUPPORTED` rather than emitting an invalid Wasm artifact.
The formatter project must still implement canonical rendering and round trips.

## Contextual noncapturing type checking (signature-only)

The Rust frontend now exposes `check::lambda_types::check_noncapturing`.
Given an expected callback parameter list (e.g. `[int]`), it can infer a
scalar result type from the lambda body or validate an explicit annotation.
It checks basic arithmetic, comparisons, local `val`/`let` bindings,
immutable reassignment, and return paths through `if`/`else` blocks.
It refuses unknown/outer variables, `self` captures, unsupported effects,
nested closures and non-scalar type annotations rather than guessing.

This is **not** a callable-value type in the executable compiler IR; no
closure objects, Rx method integration, ownership transport, callback
invocation or Wasm closure lowering are implemented. The normal `compile()`
path continues to reject lambda values with `E_UNSUPPORTED` until those
capabilities have been tested independently.

## Audit hardening: unreachable tails and nested statements

The signature-only lambda analyzer now rejects any statement after a
provably terminating return or an `if` whose every arm returns, using
`E_UNREACHABLE`. It does not silently ignore captures, mismatched returns,
or unimplemented effects in such tails. A bounded statement-depth budget
applies independently of expression-depth limits, including ASTs constructed
directly by tooling rather than via the parser. The `self` binding is also
prohibited at the type-checker boundary. These are intentionally conservative
checks: they do not grant executable closures or a Wasm callable ABI.

## Direct-call execution milestone (limited browser Wasm profile)

A single local immutable lambda per ordinary function can now be invoked
directly in the same function, provided its first call uses scalar literal
arguments. The compiler infers argument types from that call, verifies the
lambda with the noncapturing type checker, and emits a private typed Wasm
function using the existing direct-call ABI.

Example: `val double = |n| : int -> n * 2; return double(21);`
executes with result `42`. An explicitly terminated lambda return must
be parenthesized, as in `val double = (|n| -> return n * 2;);`.

The local lambda is a compile-time declaration, not a runtime handle.
Only direct calls are supported: callable values, aliases, escaped closures,
mutable / non-scalar captures, reassignments, multiple lambdas per source function,
contextual inference from nonliteral first arguments, and Rx pipelines still
fail closed. The limited direct-call path now supports immutable scalar captures
from top-level locals or immutable parameters; their values become hidden typed
arguments to the lifted function. Captures are restricted to expression-bodied
lambdas and the compiler never creates a first-class callable or heap environment.
The direct-only path itself does not produce escaped callable environments;
a separate, restricted factory-return path is documented below. The caller's scalar captures are
immutable, so invocation-time passing preserves creation-time capture semantics.
This milestone uses ordinary typed Wasm direct calls, not indirect-call tables
or a general first-class function type. Positive/negative guest fixtures and
native-vs-Wasm byte parity protect the admitted subset.

## Direct-call scope audit

Direct-call desugaring now preserves the same-scope local-name history before
removing the callable declaration, preventing duplicate bindings from being
laundered through a compiler-private generated function. An existing lexical
binding in an earlier *nested* block remains independent. Shadowing the
callable after declaration remains unsupported. Forged direct ASTs cannot
claim generated private function names, and recursive AST rewrite walks have
independent expression and statement nesting limits. These protections apply
to the direct-call-only subset; callable values and capture semantics remain
separate milestones.

## Typed callable-signature IR (pre-indirect dispatch)

The Rust checker now exposes a bounded scalar `CallableSignature` and a
validated `TypedCallable` descriptor in `check::callable_ir`. The existing
direct-lambda desugaring now constructs that descriptor for a private
function index and checks its invocation signature before handing the
desugared function to the normal checker and Wasm backend. Argument arity,
parameter types, supported scalar profiles and module-function bounds are
validated centrally. This does not add observable changes to guest Wasm
function indexing or the existing direct-call contract.

The `CallableTarget::Indirect` variant is a **reserved IR intent**, not an
implemented feature: construction fails with `E_UNSUPPORTED` until a
per-instance Wasm table ABI, stale handle checks, ownership and type-index
validation are implemented. Callable parameters/returns in source syntax,
first-class local function values, closure captures and Rx callbacks are
still unsupported. The types cannot be interpreted as runtime handles.

## Source callable types: parsed, validated, and gated

The browser frontend now recognizes scalar callable type syntax such as
`fnc(int) -> int`, `fnc(int, bool) -> string`, and `fnc() -> void`.
For example, a declaration may use `fnc(int) -> int callback` as a
parameter type, or `val callback: fnc(int) -> int = ...` as an
annotated binding. Identical scalar signatures are interned by canonical
shape (including aliases such as `boolean` / `bool` and
`String` / `string`), not by source position.

The parse/check boundary enforces parameter count and scalar-only types,
resolves canonical signatures into `CallableSignature`, and rejects
forged or malformed signature metadata. **Executable callable parameters,
values, returns and indirect dispatch remain unsupported:** a program
that needs those runtime features fails with `E_UNSUPPORTED` and produces
no guest Wasm. `fnc(int) -> int` describes a callable contract; it
does not yet allocate a function table or make lambdas first-class.

The earlier direct-lambda profile (private, noncapturing direct Wasm calls)
continues to work unchanged. The next milestone is verified per-instance
Wasm table entries and a typed indirect-call ABI with ownership checks.

When a lambda is explicitly annotated with a callable type, the
compiler performs contextual arity, return-type, and capture analysis
before returning the runtime `E_UNSUPPORTED` gate. For example, an
annotated `fnc(int)->bool` lambda returning an integer produces
`E_TYPE`, while referring to an outer variable produces `E_CAPTURE`.
A correctly typed annotation still cannot execute or escape without the
indirect-table ABI.

## Playground direct-lambda / functor compatibility profile (candidate)

The browser-oriented `closure-lambda-functor-direct-profile.atto` guest
fixture composes three **independently compiled direct** paths: a capture-free
scalar lambda, one expression-bodied lambda capturing an immutable scalar
parameter, and an ordinary class with an explicit `.call(int)` method. It
returns 42 if all three behave correctly. `tests/closures-wasm.mjs` must
compile that same source via both the native Rust compiler and the actual
`wasm32-unknown-unknown` compiler, compare guest module bytes, and execute
the generated guest to observe `main() == 42`. The fixture is not a claim
that callable objects are generally executable.

A lifted captured name must retain **definition-site lexical identity**.
Until the compiler has an owned closure environment, nested bindings,
for-in binders, and match-arm binders that shadow a lifted capture are
rejected with `E_CAPTURE`, rather than silently redirecting the read to
the inner binding. The corresponding negative source fixture is
`lambda-direct-capture-shadow.atto`. Mutable captures and escaping factory
closures are separate negative tests.

Reference comparisons:

- `oreslang-source.java/examples/callables.ores` demonstrates lexical
  closure capture versus explicit `nlex` noncapturing lambdas; its
  `OwnershipAndClosureTest` also exercises escaping mutable captures.
- `oreslang-source.llvm` currently contains scalar frontend/IR and
  experimental functor type and runtime components, not a proven
  native guest implementation of the full escaping closure/functor ABI.
- In AttoLang, the canonical authored lambda return annotation uses
  `|n| : int -> ...`. `fnc(int) -> int` names a callable **source type**
  but does not automatically authorize indirect Wasm calls.

The website's broader **Closures, functors and lambdas** preview also uses
`make_adder(...) -> fnc(int) -> int` and a higher-order
`apply_twice(callback, ...)`. These require first-class callable values,
owned closure environments, a verified typed indirect-call/table contract,
and native/wasm32/browsers guest execution receipts. **Do not enable the
preview's Run/Format actions or promote compiler bytes based only on this
bounded direct-call fixture.** The currently pinned public compiler predates
this branch and must be separately rebuilt, independently reviewed, and
released through the protected Pages gate.

## First executable factory-escaped closure (owned int environment only)

The Rust compiler now implements one **bounded** callable factory profile:
an ordinary `fnc` returns the source type `fnc(int)->int`, and its **only**
statement returns an expression-bodied lambda `|n| : int -> ...`.
The lambda may capture up to sixteen **immutable `int` factory parameters**
and use scalar pure operators; nested lambdas, mutable/borrowed/object captures,
unknown or subsequently declared bindings, effects and arbitrary lambda
statements remain unsupported.

```atto
fnc make_adder(int offset): fnc(int) -> int {
  return |n| : int -> n + offset;
}
pub fnc main(): int {
  val add_one = make_adder(1);
  val add_ten = make_adder(10);
  return add_one(21) + add_ten(10); // 42
}
```

The compiler lowers the callable into a **non-source-spellable, nominal
private class** (`$atto_escaping_closure_0`) with copied `int` capture
fields. Each factory invocation uses the existing bounded guest Wasm heap
allocator and checked class constructor, so its environment survives
the factory's stack frame and has a separate identity from other invocations.
`f(n)` on a directly bound immutable local of that factory becomes a
statically resolved and signature-checked `f.call(n)`. No unvalidated Wasm
function-table indices, host pointers or foreign `funcref` values are exposed.
The class's private capture fields are inaccessible from ordinary source.

The environment has **guest-instance lifetime** in the current
fixed-size bump heap: allocation is bounded and traps on exhaustion, but
there is **no individual closure destructor or heap reclamation**. This is
not a claim of real-time bounded-allocation latency, independent-closure
release, arbitrary first-class callable values, portable `call_indirect`,
generics, ownership-qualified callable transport, mutable captured state
or Wasm/LLVM/Java complete parity.

**Fail-closed boundary:** only one escaped factory per module is admitted;
only local immutable `val` bindings directly initialized from that
factory may use `f(n)`. Passing functions as arguments or returning a
different callable, aliasing callable values, synthetic-class construction
from source, cross-actor transport and unsupported signatures remain rejected
by the parser/checker before guest Wasm emission. The generated class name
begins with `$`, which cannot be a source identifier. Invocation uses the
normal class ownership checker and forbids access to private fields, use
after move, mutable-capture escapes, shadowing and wrong-typed arguments.

**Execution gate:** `tests/fixtures/pass/closure-escape-*.atto`,
the exact native Rust/Wasm compiler comparison in
`tests/closures-wasm.mjs`, the complete guest conformance suite and real
Chromium/Web Worker E2E must pass against an exact Git source tree before
considering this profile verified. In particular, the full website
`apply_twice(fnc(int)->int, ...)` example stays **non-executable** until a
separate owned first-class callable and typed Wasm indirect-dispatch ABI is
implemented, reviewed and published. Compiler bytes and release receipts
must not be silently updated by this draft.

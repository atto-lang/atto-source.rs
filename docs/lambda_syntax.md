# AttoLang lambda return-type syntax (parser profile v1)

The preferred braceless form is `|parameters| : ReturnType -> body`.
The annotation is optional: return types should normally be inferred from
expressions and contextual callable signatures (when the relevant backend
supports closure typing). Use `=>` only in **callable type definitions** such as `fnc(int) => int`;
use `->` between a lambda's parameters/return annotation and its body.

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
those nested inside method-call arguments. Most syntax remains preview-only. The Rust browser compiler now admits a
**bounded executable** `fnc(int) => int` profile using private class-backed
immutable integer captures and a one-entry typed Wasm indirect-call table.
General callable handles, arbitrary capture types, nested closure bodies,
memory reclamation, actor transport and Rx callbacks remain unsupported and
must fail closed. No frontend syntax support alone proves execution.
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
Factory-returned closures remain unsupported. The caller's scalar captures are
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
validation are implemented. The scalar `fnc(int) => int` class-backed profile is lowered separately;
the reserved IR target is **not** a general runtime handle representation.
Other callable signatures and Rx callbacks remain unsupported.

## Source callable types: parsed, validated, and gated

Use `=>` for callable **type declarations**, such as `fnc(int) => int`.
Use `->` for lambda **body boundaries**, such as `|n| : int -> n * 2`.
The former `fnc(int) -> int` type spelling is rejected, not silently accepted as an alias.

The browser frontend now recognizes scalar callable type syntax such as
`fnc(int) => int`, `fnc(int, bool) => string`, and `fnc() => void`.
For example, a declaration may use `fnc(int) => int callback` as a
parameter type, or `val callback: fnc(int) => int = ...` as an
annotated binding. Identical scalar signatures are interned by canonical
shape (including aliases such as `boolean` / `bool` and
`String` / `string`), not by source position.

The parse/check boundary enforces parameter count and scalar-only types,
resolves canonical signatures into `CallableSignature`, and rejects
forged or malformed signature metadata. The **narrow** `fnc(int) => int` profile now supports first-class local
closures, callable parameters/returns, and a checker-generated typed Wasm
indirect call through a sealed one-entry method table. An actual owned integer
capture survives its creating function's return. Other callable signatures
still fail closed; this is **not** the final general callable-handle ABI.

The earlier direct-lambda profile (private direct Wasm calls) continues to
work unchanged. The next milestone is **general** per-instance function
table entries, owned reclaimable capture frames, generation-fenced handles,
actor/domain validation, and cleanup on all exceptional exits.

When a lambda is explicitly annotated with a callable type, the
compiler performs contextual arity, return-type, and capture analysis
before returning the runtime `E_UNSUPPORTED` gate. For example, an
annotated `fnc(int) =>bool` lambda returning an integer produces
`E_TYPE`, while referring to an outer variable produces `E_CAPTURE`.
A correctly typed `fnc(int) => int` lambda may execute or escape when
used in the newly supported bounded profile; uninvoked standalone local
annotations still follow the existing conservative gate until the full
ownership/lifetime analysis is integrated.

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
`lambda-direct-capture-shadow.atto`. Mutable captures and unsupported escaping closure bodies remain separate
negative tests; an expression-bodied factory with immutable integer captures
is now executable.

Reference comparisons:

- `oreslang-source.java/examples/callables.ores` demonstrates lexical
  closure capture versus explicit `nlex` noncapturing lambdas; its
  `OwnershipAndClosureTest` also exercises escaping mutable captures.
- `oreslang-source.llvm` currently contains scalar frontend/IR and
  experimental functor type and runtime components, not a proven
  native guest implementation of the full escaping closure/functor ABI.
- In AttoLang, the canonical authored lambda return annotation uses
  `|n| : int -> ...`. `fnc(int) => int` names a callable **source type**
  but does not by itself authorize arbitrary first-class Wasm handles.

The website's broader **Closures, functors and lambdas** preview also uses
`make_adder(...): fnc(int) => int` and a higher-order
`apply_twice(callback, ...)`. The bounded Rust/Wasm compiler now executes the scalar `fnc(int) => int`
subset (including `make_adder` and `apply_twice`) through a private typed
indirect method table, but full general-purpose closure ownership, cleanup,
and browser execution remain unverified. **Do not enable the preview's
Run/Format actions or promote compiler bytes based on this limited subset.** The currently pinned public compiler predates
this branch and must be separately rebuilt, independently reviewed, and
released through the protected Pages gate.

## Executable class-backed owned integer closures (draft)

The source compiler currently desugars `fnc(int) => int` to an internal,
unnameable class with one dispatch selector and four integer capture slots.
Closure creation takes **value snapshots** of immutable `int` bindings into
a module-private bump heap. A generated `.call(int)` method dispatches among
a bounded set of expression-bodied targets. The Wasm emitter installs that
method in an unexported `funcref` table and uses **typed `call_indirect`**
at the invocation site. This is an executable closed-world adapter, **not**
the production generational `CallableHandle` ABI.

The initial caps are one `int` input, one `int` result, at most four
immutable `int` captures per lambda, at most 32 lambda sites, and simple
expression bodies. Source functions with unrelated control flow continue
through the ordinary checker. The guest heap currently has bounded bump
allocation **without reclaiming closed closure environments**; do not rely
on this prototype for unbounded allocation or hard-real-time lifetimes.

Exact-source CI acceptance fixtures:
- `callable-owned-make-adder.atto` returns 42 after its factory returned
- `callable-owned-apply-twice.atto` returns 40 through a higher-order argument
- `callable-owned-independent-captures.atto` returns 43
- `callable-owned-multiple-targets.atto` returns 41
- `callable-owned-unrelated-control-flow.atto` returns 42
- malformed type arrows, mutable captures, unsupported bodies and use-after-move
  reject before emitting a guest module

The remaining release gates are the entire exact Git source tree and test
corpus, real Chromium Worker execution, sanitizer evidence, production capture
cleanup, stable ABI documentation and backend-parity review.

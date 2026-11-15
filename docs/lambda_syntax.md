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
those nested inside method-call arguments. This is **syntax-only**: the Rust
browser compiler does not yet implement callable value types, capture/borrow
checking, callback invocation, Rx Observable methods or Wasm closure lowering.
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
captures, reassignments, multiple lambdas per source function, contextual
inference from nonliteral first arguments, and Rx pipelines still fail closed.
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

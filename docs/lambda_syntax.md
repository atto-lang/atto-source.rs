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

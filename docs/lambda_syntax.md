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
inside a call argument before the closing `)`. Expression bodies do not need
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

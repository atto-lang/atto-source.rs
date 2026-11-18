# Draft: type-first callable return syntax (U+2022 or ASCII <-)

AttoLang may use a return type followed by either the exact Unicode BULLET character U+2022 or contiguous ASCII `<-`. The `fnc` keyword is optional only for these explicitly delimited variants:

~~~atto
fnc foo(): int { return 42; }    // established
fnc int• foo() { return 42; }    // new
int• foo() { return 42; }        // new
int <- foo() { return 42; }      // equivalent ASCII spelling
fnc int <- foo() { return 42; }  // optional fnc keyword
int<-foo() { return 42; }        // whitespace not required
~~~

The parser normalizes these spellings to the same return type and callable AST. The bullet is a single lexer token. To preserve existing expression semantics (`x<-1` means `x < -1`), `<-` is recognized as two adjacent existing tokens **only in a type-first callable header**; whitespace or comments may surround but must not split the two ASCII characters (`< -` is not accepted). Strings and comments retain their ordinary behavior.

## Redundant and contradictory postfix annotations

~~~atto
int• add(a: int,b: int): int { return a+b; }     // W_DUPLICATE_RETURN_TYPE
int• add(a: int,b: int): bool { return a+b; }    // E_RETURN_TYPE_CONFLICT
int• add(a: int,b: int): !int { return a+b; }    // E_RETURN_EXCLUSION
~~~

The leading return type is authoritative for either delimiter. A matching postfix type is a warning, not an error. The exclamation mark here denotes the type complement (everything other than T), not ownership, take or nullability. Because T intersect !T is empty, this is a compile error. The browser executable subset rejects other complement types pending a sound type-set implementation.

Generic language-level spelling is T• identity<T>(value: T). The browser compiler currently lacks generic monomorphization; such signatures are rejected with E_UNSUPPORTED, though direct contradictions with !T are diagnosed first. Passing parser tests for nested supported ADT spellings are not proof of generic executable support.

## Test and rollout notes

The Rust frontend tests cover postfix, bullet and ASCII `<-` spellings, top-level/class/actor/static contexts, nested built-in type spellings, warnings, conflicts, exclusions, malformed source, Unicode strings/comments, type checking, no-warning state leakage, and deterministic emitted Wasm. A new compile_with_warnings entry point reports non-fatal diagnostics, and the native CLI prints these to stderr. The current wasm browser ABI only exposes success/error: surfacing warnings in the playground requires a follow-up ABI/UI change. Cross-backend parity and generic code generation remain separate gates.


## Newline/end callable bodies (type-first syntax)

The brace-free type-first form has a **mandatory physical newline** after the
complete declaration signature (including any postfix annotation/effect
clauses). Indentation and column position do not determine the block. The
callable's own `end` may be followed by exactly one optional `.` or `;`.

~~~atto
int <- foo()
    return 42;
end.

int• bar()
    return 42;
end;

fnc int <- baz()
    return 42;
end

fnc int• qux()
    return 42
end.
~~~

Braced equivalents remain accepted, including same-line bodies. These
semicolon/newline-separated statement bodies reuse the checked explicit/end
parser; a trailing expression is **not** an inferred return. Nesting is based
on the actual grammar (`if ... fi`, `match ... end`, and class/actor
`end` delimiters each consume their own closing token). Missing closing
`end`, a same-line unbraced statement, an empty body, a second end
punctuation mark, a conflicting return annotation, and an unsupported
Unicode BLACK CIRCLE U+25CF delimiter must fail closed.

The spellings `<-` and U+2022 `•` are the only two admitted prefix
return delimiters; U+25CF `●` is deliberately excluded. This change
does not add general newline-delimited bodies to legacy headers or alter
`x<-1` expression lexing. Real compiler execution and other backend
conformance remain rollout gates.

## Explicit begin/end callable bodies

A type-first callable may put `begin` after the signature to make
the body boundary explicit. The usual checked statement grammar parses
the body and its matching `end`. The closer can be written as
`end`, `end.` or `end;` (exactly one optional punctuation mark).
Indentation has no grammar effect; the formatter uses two spaces.

~~~atto
int <- foo()
begin
  return 42;
end.

int• bar()
begin
  return 42;
end.

fnc int <- baz()
begin
  return 42;
end;

fnc int• qux()
begin
  return 42;
end
~~~

An inline class/actor instance method may use `mtd int• name()`
or `mtd int <- name()` in addition to legacy `fnc` and bare
method headers; `mtd` remains invalid as a top-level function keyword.
`begin/end` does not imply a final-expression return. Unmatched
`end`, duplicate closer punctuation, empty bodies, and contradictory
return types fail closed. General nested begin/end expressions and other
backend parity are separate work.

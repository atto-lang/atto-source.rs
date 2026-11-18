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

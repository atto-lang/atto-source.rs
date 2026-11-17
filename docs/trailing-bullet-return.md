# Draft: trailing bullet return-type syntax (U+2022)

AttoLang may use a return type terminated by the exact Unicode BULLET character U+2022. The fnc keyword is optional only for this explicitly delimited variant:

~~~atto
fnc foo(): int { return 42; }    // established
fnc int• foo() { return 42; }    // new
int• foo() { return 42; }        // new
~~~

The parser normalizes all three to the same function return type. The lexical delimiter is not an identifier character; comments, whitespace and strings continue to follow the ordinary UTF-8 lexer.

## Redundant and contradictory postfix annotations

~~~atto
int• add(a: int,b: int): int { return a+b; }     // W_DUPLICATE_RETURN_TYPE
int• add(a: int,b: int): bool { return a+b; }    // E_RETURN_TYPE_CONFLICT
int• add(a: int,b: int): !int { return a+b; }    // E_RETURN_EXCLUSION
~~~

The bullet return type is authoritative. A matching postfix type is a warning, not an error. The exclamation mark here denotes the type complement (everything other than T), not ownership, take or nullability. Because T intersect !T is empty, this is a compile error. The browser executable subset rejects other complement types pending a sound type-set implementation.

Generic language-level spelling is T• identity<T>(value: T). The browser compiler currently lacks generic monomorphization; such signatures are rejected with E_UNSUPPORTED, though direct contradictions with !T are diagnosed first. Passing parser tests for nested supported ADT spellings are not proof of generic executable support.

## Test and rollout notes

The Rust frontend tests cover old and new spelling, top-level/class/actor/static contexts, nested built-in type spellings, warnings, conflicts, exclusions, malformed source, Unicode strings/comments, type checking, no-warning state leakage, and deterministic emitted Wasm. A new compile_with_warnings entry point reports non-fatal diagnostics, and the native CLI prints these to stderr. The current wasm browser ABI only exposes success/error: surfacing warnings in the playground requires a follow-up ABI/UI change. Cross-backend parity and generic code generation remain separate gates.

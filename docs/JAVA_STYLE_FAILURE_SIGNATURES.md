# Java-style failure effects and callable body delimiters (draft compiler contract)

This document describes the proposed browser compiler admission slice, stacked on
`atto-source.rs#301`. It does not claim that arbitrary user-defined `Error`
subclasses, priority actor CONTROL transport, or actual `raise`/`panic` unwinding
are executable in the browser target.

## Signatures

All forms below are valid *syntax* for functions and class/actor methods:

```atto
pub fnc load_config(path: String) -> Config
    throws IOError, ParseError
    raises RecoveryError
begin
    return parse_config(read_file(path));
end

pub fnc load_config(path: String)
    throws IOError, ParseError
    raises RecoveryError -> Config
    return parse_config(read_file(path));
end

pub Config• load_config(path: String)
    throws IOError, ParseError
    raises RecoveryError {
    return parse_config(read_file(path));
}

@Throws(IOError, ParseError)
@Raises(RecoveryError)
pub fnc load_config(path: String) -> Config {
    return parse_config(read_file(path));
}
```

- An inline `throws`/`raises`/`panics` list **after** `-> T`, `: T`, or
  `T•` requires `{` or `begin`. An implicit end-only body is rejected.
- If the return type is **after** inline effects (`raises E -> T`),
  `{...}`, `begin...end`, or a real newline followed by
  statements and `end` is accepted. Comments/blank lines do not change
  the physical newline requirement.
- `begin...end` uses ordinary statements and semicolon/newline
  terminators, **not** Erlang comma-tail implicit-return semantics.
- Prefix annotations do not themselves trigger the explicit body-opener
  requirement. Existing legacy non-effect signatures retain their body forms.
- A return type may appear **once**, not both before and after effects.
- Mixed annotation/inline declarations of the same effect kind are rejected.
- Multiple different effects are independent; `throws` does not handle `raises`
  or `panics`.

## Safety boundaries

This compiler subset currently accepts only the built-in `Error` exception
type. `throws IOError` and `raises RecoveryError` are recognized syntax but
fail with an explicit unsupported-type diagnostic pending nominal Error-subclass
resolution. The compiler must never treat those names as built-in `Error` or
erase them from the checked effect contract. `@Throws(Error)` and
`@Raises(Error)` normalize to the same effect flags as inline clauses.

The full language specification should retain the declared **individual error
types**, not just the boolean throw/raise/panic mask used by this early browser
subset. This browser change is not a proof that every potential exception is
handled, nor evidence of parity in native/other backends.

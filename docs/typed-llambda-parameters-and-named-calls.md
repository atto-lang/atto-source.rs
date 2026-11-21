# Typed llambda parameters and named call arguments (Rust/Wasm candidate)

Status: **implementation draft; exact-head execution and review required**.

## Surface forms

Both of these declarations specify the same typed function ABI and formal
names; parameter style is never part of overload identity.

```atto
const fnc c = |int a, int b| : int -> a + b;
const fnc c = |a: int, b: int| : int -> a + b;
```

An Atto source file cannot declare both under the same name and arity: normal
duplicate declaration diagnostics apply. Mixed styles are accepted.

```atto
const fnc sum = |int a, b: int| : int -> a + b;

pub fnc main(): int {
    return sum(b: 22, a: 20);
}
```

* `|a,b| -> ...` remains accepted when callable context or the first bounded
  literal-only direct call can determine the argument types.
* `|int a, b: int| : int -> ...` stores a canonical explicit parameter
  signature; it no longer requires literal-only inference for every parameter.
* `const fnc` at module scope is a statically named, noncapturing, immutable
  declaration lowered to the existing ordinary function ABI. All parameter
  types and the result must be explicit, and currently must be scalar.
* A local `const fnc` is a closed direct-call lambda with mandatory
  `|...|` delimiters. Parameter types may be explicit or inferred from the
  first verified scalar-literal direct invocation. Example:
  `const fnc v = |_| : int -> 5; return v(41);` infers one `int` input;
  `const fnc v = || : int -> 5; return v();` has zero inputs. The `: int`
  annotates the *result*, never an implicit parameter type.
  The local constant may not capture runtime locals or escape as a value.
  Unused or uninferrable callables still fail closed. This is **not** a
  general first-class callable or escaping closure.
* Module-scope `const fnc` declarations still require explicit parameter
  annotations (use `|int _|` for an ignored `int` input); their ABI is fixed
  before call-site analysis. A zero-argument `||` needs no input annotation.
* Delimiter-free lambda heads (`_ : int -> 5`, `_ -> 5`, `x -> x + 5`) are
  syntax errors, not alternative lambda spellings.

## Named calls

Named arguments use `name: expression` and refer to *formal parameter names*,
not keyword maps or record fields. The parser admits positional arguments
before named arguments; the checker resolves formal names statically.

```atto
sum(20, b: 22);
sum(b: 22, a: 20);
```

The checker rejects unknown, duplicate, or missing names, mismatched argument
types, and excess argument count. Named binding is available for ordinary
functions, direct scalar llambdas, class methods, actor-local methods, and
constructors that otherwise pass their established isolation and type gates.
It does not bypass visibility, ownership, effects, or actor isolation rules.

**Evaluation order is observable.** Until explicit source-order temporary slots
exist in typed IR, a named call that changes argument order is admitted only
if every argument expression is a scalar literal. Named calls already in
formal order may contain ordinary expressions, evaluated exactly once.
Rejecting an unsupported reorder is safer than silently changing effects.

## Remaining gaps

Arbitrary dynamic callable dispatch, escaping closure environments, generic
callable ABI, defaults/omittable formals, named calls for built-in host
capabilities without declared formal names, and cross-backend parity are not
certified by this Rust/Wasm slice. Formatter canonicalization and shared
cross-backend fixtures must be reconciled in a separate spec/conformance PR.

For private CI, `native/.github/private-ci-approved-repositories.txt` has no
approved external export destinations. Source must not be mirrored into an
unapproved organization; a release claim requires actual executed jobs on an
authorized runner plus exact source-head receipts.

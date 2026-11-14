# Declarative exports (AttoLang browser frontend)

Exports are file-level declarations. They can appear **before**, **between**, or
**after** the declarations they refer to; a full-file name-resolution pass checks
them after parsing. They do not define classes, functions, or values.

```atto
export pub Foo;
export pub[pkg] class Bar;
export pub fnc calculate;

define class Foo as
end

define class Bar as
end

fnc calculate(int x): int { return x + 1; }
pub fnc main(): int { return calculate(41); }
```

Equivalent placement:

```atto
define class Foo as
end

define class Bar as
end

export pub class Foo;
export pub[pkg] Bar;
```

## Grammar

```ebnf
export_decl = "export", "pub", [ "[", "pkg", "]" ],
              [ "class" | "fnc" | "actor" | "enum" ], identifier, ";" ;
```

* `export pub Foo;` infers the declaration kind from the current file.
* `export pub class Foo;` and `export pub fnc calculate;` check the
  explicitly supplied kind. `actor` and `enum` are also recognized by this
  frontend. Additional declaration categories need their own parser/checker
  support before they are exportable.
* `pub` makes the name publicly visible to importing packages; `pub[pkg]`
  limits visibility to the defining package. It is not a property of the
  declaration's position in the source.
* A function name exports all local arity overloads of that function.
* Unknown names, wrong declaration kinds, and duplicate exports are errors.
  A target must be declared in the same file in this frontend stage; re-exports
  across imported modules are not implemented.
* The `;` after an export is mandatory, so a missing separator cannot
  silently consume the following declaration.
* An `export` statement is not a runtime expression and cannot appear inside
  a function, class, actor, or other nested scope.

## Implementation and limits

The browser compiler resolves and type-checks these entries, then writes a
deterministic **`atto.exports.v1`** WebAssembly custom section. This is
language-level module metadata: a sequence of length-prefixed names followed
by one kind byte (class=1, fnc=2, actor=3, enum=4) and one visibility byte
(pub=1, pub[pkg]=2). Entries are sorted by name.

The single-file browser executable retains its existing Wasm exports and
`main()` requirement. This metadata **does not yet implement multi-file
package linking or enforce package boundaries at import time**; the native
compilers and future module linker must consume the same visibility contract.
In particular, `pub[pkg]` must never become a public host Wasm export.

## Required cross-backend parity

Java, LLVM, and .NET frontends should parse the same grammar, resolve exports
only after collecting all top-level declarations, reject kind/visibility
mismatches and duplicates, preserve module export tables in IR, and enforce
`pub[pkg]` on actual imports. Add forward-reference, overload, namespace,
cycle, and cross-package negative fixtures in each backend's integration suite.

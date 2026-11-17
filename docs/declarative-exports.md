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
              [ "class" | "fnc" | "actor" | "enum" | "module" ], identifier, ";" ;
```

* `export pub Foo;` infers the declaration kind from the current file.
* `export pub class Foo;` and `export pub fnc calculate;` check the
  explicitly supplied kind. `actor` and `enum` are also recognized by this
  frontend. Additional declaration categories need their own parser/checker
  support before they are exportable.
* `export pub module Name;` and `define pub module Name as ... end` select the same public module interface; a module export does **not** expose private members.
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

The browser compiler collects file-level export directives during lexical lowering, resolves them against the complete parsed file and module declaration table, carries them through the checked AST/IR, then writes a
deterministic **`atto.exports.v1`** WebAssembly custom section. This is
language-level metadata, not a host Wasm export. A custom section contains a sequence of length-prefixed names followed
by one kind byte (class=1, fnc=2, actor=3, enum=4) and one visibility byte
(pub=1, pub[pkg]=2). Entries are sorted by name. Export names are
restricted to **canonical ASCII identifiers of 1–256 bytes** at the compiler
boundary, even though unexported source identifiers may contain Unicode.
The compiler diagnoses non-ASCII names as `E_EXPORT_NAME` and overlong names
as `E_EXPORT_NAME_TOO_LONG`, rather than emitting a Wasm manifest the host
would reject.

Named modules use a separate **`atto.exports.modules.v1`** local metadata section, containing module name and visibility, because the established `atto.exports.v1` kind byte table has no module kind. The compiler carries all resolved export kinds through one checked AST/IR export table before serialization. A future package/namespace linker must adopt a versioned identity-aware ABI; the existing module section is **not** a cross-file linking contract.

`internal fnc` is file-local and cannot be promoted via `export`. Unlinked `external fnc` prototypes cannot be called or exported. `protected` requires real subclass access analysis and currently rejects rather than silently translating to `private`. `final` is field-level single-assignment, not a visibility declaration. See the [visibility and linkage specification](https://github.com/atto-lang/atto-spec/pull/68) (draft PR #68 until merged).

The single-file browser executable retains its existing Wasm exports and
`main()` requirement. This metadata **does not yet implement multi-file
package linking or enforce package boundaries at import time**; the native
compilers and future module linker must consume the same visibility contract.
In particular, `pub[pkg]` must never become a public host Wasm export.

## Verified subset and missing linker

A SHA-bound, source-only private Actions carrier exercised 26 Rust browser-compiler tests at commit `b5e8b45827ff0a7f41eba7d32440720007e44fa6`; later commits require fresh exact-head validation. All 33 copied compiler/test blobs were independently matched to their source Git hashes. This does not establish the full repository test suite, native backend parity, or package linker behavior.

`use namespace` and cross-file `import module ... from ...` remain unsupported in this browser build and must fail closed.

## Required cross-backend parity

Java, LLVM, and .NET frontends should parse the same grammar, resolve exports
only after collecting all top-level declarations, reject kind/visibility
mismatches and duplicates, preserve module export tables in IR, and enforce
`pub[pkg]` on actual imports. Add forward-reference, overload, namespace,
cycle, and cross-package negative fixtures in each backend's integration suite.

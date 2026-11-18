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

The standalone `compile(source)` API remains single-file. The
`linker::compile_linked` API now supports **bounded, multi-hop dependency graphs**
of explicitly supplied sources with `./` and package-root `+` imports,
per-edge `pub[pkg]` validation, cycle diagnostics, no implicit re-exports,
and checks that each file explicitly owns or imports qualified module names.
A source file cannot acquire a module through a sibling's import.

The host-side `package_resolver::compile_package` API, and the native
`attoc --package-root DIR --entry src/main.atto -o out.wasm` command, accept
a strict `.atto.pkg.toml` subset (`[package]`, `name`, `version`),
canonicalize local source paths and reject symlinks and out-of-root source
paths. The resolver loads only reachable files and passes their verified
single-root identity to the linker.

See [checked package linking](package-linking.md) for concrete imports,
exact syntax, import graph budgets, threat model and deliberate limitations.
The filesystem resolver does **not** yet authenticate cross-package registry
dependencies or protect against races in concurrently mutable hostile trees.
For that, immutable input snapshots or race-safe descriptor-based resolution
are required. The in-memory API still requires trusted package IDs supplied
by its caller. Full external dependency linking, aliases, native backend parity,
and AOT package-qualified identity metadata remain outside this subset.

## Required cross-backend parity

Java, LLVM, and .NET frontends should parse the same grammar, resolve exports
only after collecting all top-level declarations, reject kind/visibility
mismatches and duplicates, preserve module export tables in IR, and enforce
`pub[pkg]` on actual imports. Add forward-reference, overload, namespace,
cycle, and cross-package negative fixtures in each backend's integration suite.

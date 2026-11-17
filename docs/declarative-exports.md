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

The standalone `compile(source)` API remains single-file and deliberately
rejects namespace/import syntax. A **separate** `linker::compile_linked` API now
accepts a bounded, explicitly supplied in-memory graph of source units and
emits one statically linked browser guest for one-hop named-module imports.
At link time it verifies the imported file actually declares the requested
exported module, checks `pub[pkg]` package identity, rejects inaccessible
members and imported re-exports, and enforces canonical relative paths and
source-size limits. It does not simply expose package symbols as Wasm host
exports.

This is NOT an on-disk package manager or complete package resolver:
`LinkUnit.package` MUST come from an already trusted resolver of the package
manifest, and the in-memory API cannot prove symlinks, physical paths, or
external package identity. Transitive imports, cycles, module aliases,
project-root `+` imports, general FFI, protected inheritance and native
backend parity remain unsupported or fail closed. Namespace headers establish
file-level ownership, not open imports or a globally usable name table.

Example:

```atto
// src/math.atto — package third.party
use namespace library;
export pub module Math;
define module Math as
  pub fnc double(int x): int { return x * 2; }
  fnc secret(): int { return 99; }
end

// src/main.atto — package application
use namespace frontend;
import module Math from "./math.atto";
pub fnc main(): int { return Math.double(21); }
```

The in-memory linker compiles these two separately supplied files into one
guest. A `pub[pkg]` export is accepted only when the provided, verified
canonical package identities are identical. `Math.secret()` is rejected.

Exact-blob runner [44164708007](https://github.com/quaestor-ledger-test/atto-ci/actions/runs/44164708007)
executed 40 Rust tests and two real Node WebAssembly guest runs on the
source-only mirror of the compiler at commit
`0c1ff72d9d8fc1f9b7097de7dd6c11606acc2eab`;
later commits need refreshed receipts. This is a scoped source-only mirror,
not the full repository test suite or cross-backend parity.

## Required cross-backend parity

Java, LLVM, and .NET frontends should parse the same grammar, resolve exports
only after collecting all top-level declarations, reject kind/visibility
mismatches and duplicates, preserve module export tables in IR, and enforce
`pub[pkg]` on actual imports. Add forward-reference, overload, namespace,
cycle, and cross-package negative fixtures in each backend's integration suite.

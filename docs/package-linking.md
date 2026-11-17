# Checked local package linking (experimental Rust/Wasm browser compiler)

This page describes the **implemented** bounded local-package linking profile
in `atto-source.rs`. It is narrower than AttoLang's full multi-package,
namespace-aware target specification in [atto-spec PR #68](https://github.com/atto-lang/atto-spec/pull/68).

## Files and invocation

```text
project/
  .atto.pkg.toml
  src/main.atto
  lib/orders.atto
  lib/math.atto
```

In this initial supported manifest subset:

```toml
[package]
name = "storefront"
version = "1.2.3"
```

Use either:

```bash
attoc --package-root project --entry src/main.atto -o app.wasm
```

Or host Rust code:

```rust
let wasm = atto_compiler::package_resolver::compile_package(
    std::path::Path::new("project"), "src/main.atto",
)?;
```

The legacy single-file `attoc input.atto [-o app.wasm]` and
`compile(source)` remain unchanged and do not implicitly access the filesystem.
The in-memory API `linker::compile_linked(entry, &[LinkUnit])` supports
explicitly supplied source units, including cross-package public imports, but
**requires a trusted caller to supply verified package identities**.

Example entry source:

```atto
use namespace storefront;
import module Orders from "+lib/orders.atto";
pub fnc main(): int { return Orders.total(21); }
```

In `lib/orders.atto`:

```atto
use namespace sales;
import module Math from "./math.atto";
export pub module Orders;
define module Orders as
    pub fnc total(int x): int { return Math.double(x); }
end
```

In `lib/math.atto`:

```atto
use namespace maths;
export pub[pkg] module Math;
define module Math as
    pub fnc double(int x): int { return x * 2; }
    fnc secret(): int { return 91; }
end
```

The linked `main()` returns 42 in a real WebAssembly guest. The Math export
is usable by Orders because both belong to the same canonical local package.
The module's private `secret` member remains inaccessible from other modules.

## Binding and security rules

- `+lib/math.atto` resolves from the verified package root; `./math.atto`
  resolves from the importing source's directory. Both are canonical,
  project-relative paths. `../`, absolute paths, empty components, backslashes,
  and unrecognized path spellings are rejected.
- The host resolver canonicalizes the package root and checks each component
  of every reachable source path. Symlinks, paths outside the physical root,
  missing source files, and non-regular files fail without emitting a guest.
- Only the entry file and recursively reachable imports are read. The import
  graph is traversed deterministically, in dependency order; cycles, duplicate
  module identities and ambiguous names are rejected.
- **Each import edge is checked against its own importing file's package ID**.
  `pub[pkg]` is not satisfied merely because the entry file belongs to a
  compatible package. There is no transitive permission grant.
- Each source can use only modules it explicitly imports or owns. Importing
  a module in another source does not grant that name to sibling files.
  Referencing an entry-local free function from an imported file also fails.
- File-local module exports and members are bound before code emission.
  Imported exports are not re-exported as language metadata or unrestricted
  WebAssembly host exports. Module ownership and member visibility remain
  separate checks.
- Current compilation requires unique module names across the linked guest
  until package+namespace-qualified generated symbols replace the bounded
  internal module name scheme. Alias syntax and name collision resolution
  are not silently guessed.
- Bounds: at most 64 supplied/reachable source units, 32 import declarations
  per file, 8 KiB manifest, 64 KiB compiled source, and strict path/identifier
  limits. Unsupported declarations fail closed.

## Intentional limitations and threat model

`[package]` with `name` and `version` is the supported **strict TOML
subset**, not a promise to accept every valid TOML document or dependency
table. External dependencies, an authenticated lockfile/registry, aliasing,
cross-package resolution through the filesystem, symlink-enabled roots,
project-wide package discovery, lazy imports, FFI, and general runtime
module initialization remain unsupported.

`compile_package` proves local physical containment and forbids symlinks in
the observed filesystem paths. **It does not guarantee race-free filesystem
reads when an attacker can change directories concurrently.** The caller
must supply a trusted, stable source tree; openat2/capability-based file
handles and immutable source snapshots are future security requirements
before supporting adversarial filesystem mutation.

Namespaces remain file ownership headers, not open-import declarations.
The existing Wasm export metadata is local guest metadata, not portable
proof of full package identity. Native LLVM/Java/.NET/Forge parity,
full-project `cargo test --locked`, final methods/classes, protected
inheritance and validated external FFI are separate merge gates.

CI tests should execute both the compiler and resulting guest, and record
the exact source/tree Git SHAs of the files compiled. A pre-step failed job
or an unexecuted fixture is not evidence of passing compiler behavior.

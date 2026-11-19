# Namespaced cross-file module linking (browser compiler preview)

This profile connects file-scoped namespace declarations to checked, explicit local module imports. It is not an ambient namespace import, a package registry client, or an implicit re-export system.

## Source example

`src/main.atto`:

```atto
use namespace storefront;
import module math from "./math.atto";

pub fnc main(): int {
    stdio.println(math.square(7));
    return 0;
}
```

`src/math.atto`:

```atto
use namespace arithmetic;

define pub module math as
    pub fnc square(int x): int {
        return x * x;
    }
end
```

Manifest at package root `.atto.pkg.toml`:

```toml
[package]
name = "storefront"
version = "1.0.0"
```

Native CLI invocation from a checkout containing this package:

```sh
attoc --package-root . --entry src/main.atto -o storefront.wasm
# Alternative: attoc --package src/main.atto -o storefront.wasm
```

The **intended** Wasm guest contract is stdout `49\n` and return value `0`. Those properties must be demonstrated by the independent Node WebAssembly guest execution test before marking this integration validated.

## Namespace and import contract

- Namespace is optional; at most one header, before all imports or other declarations.
- Explicit dotted `use namespace shop.frontend;` is allowed.
- `use default namespace;` resolves to the containing source folder.
- `use default namespace(2);` resolves to the two enclosing relative folder components in parent.child order, and fails if too shallow.
- `import module math from "./math.atto";` is case-sensitive and never equates namespace declaration with import authority.
- `./` resolves relative to the importing file; `+` resolves at the verified package root. Both reject traversal and aliases.
- The dependency must own the imported module and explicitly export it (`define pub module` or matching `export pub module`). Package-private `pub[pkg]` imports require a matching authenticated package identity.
- Module members must be `pub` to cross an import boundary. Import cycles, ambiguous source ownership, missing files, forged aliases, private members, and ambient re-exports fail closed.
- The in-memory linker needs caller-authenticated package identities. Filesystem and locked snapshot entry points supply stronger source identity checks; they do not provide an authenticated remote package registry.
- Package linking is currently a host-side operation. The existing single-file browser Wasm compilation entrypoint does not yet accept a multi-file source graph. Playground UI integration requires an explicit authenticated source-snapshot interface and host ABI.

## Regression gates

`cargo test --locked --test namespace_storefront` verifies exact syntax and negative visibility/path conditions.

`cargo run --locked --example namespace_storefront_guest -- /tmp/storefront.wasm` creates a genuine linked guest, and `node tests/namespace_storefront_wasm.test.mjs /tmp/storefront.wasm` checks imported host function signatures, printed output, return value, and lack of accidental module host exports.

`cargo test --locked --test package_resolver --test package_cli --test multihop_linker` tests the package and transitive import boundaries.

The full suite and actual guest execution are required to claim integration validation; a GitHub workflow rejected before runner admission is **not** a test result.

## Compatibility and roadmap

This is a narrow integration port of the checked linker in atto-source.rs PR #272 onto the newer mainline namespace parser. It deliberately does not replace the current parser, typed collection lowering, actor interfaces, or other mainline features.

Next: obtain a working CI runner; ensure full suite + guest execution; then merge this focused change and wire an authenticated multi-file compilation path into the playground.

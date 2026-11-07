# Top-level define declarations (browser compiler M1 subset)

The browser subset supports:

- `define class Name as ... end`: empty declarations or `static fnc` methods only.
- `define actor Name as ... end`: empty declarations or pure static methods only. **No actor instance, spawn, mailbox, ownership sandbox or scheduler** is implemented. Actor instance members reject explicitly.
- `define enum Name as Variant, Other end`: integer-backed **nominal** variants. Qualified expressions `Name.Variant` and enum-typed params/bindings/returns are allowed. Cross-enum or enum/int arithmetic is rejected by the type checker. The declaration must precede type annotations that reference it. Payloads/methods are not supported yet.
- `define module Name as fnc ... end`: qualified module functions.
- `define struct/interface/trait Name as end`: empty nominal declarations; fields/methods/contracts reject explicitly.
- Both `define pub class` and `pub define class` are accepted, as are braced type bodies.
- `define fnc` works as an alias for `fnc` at top level.

These declarations are **not** full AttoLang actor, ADT or object semantics. They compile to an executable Wasm guest only when all used constructs are lowered; unsupported member bodies return E_UNSUPPORTED and zero guest bytes. The guest ABI remains v1; compiler ABI remains v2; no additional host imports.

Run `cargo test --offline --test top_level_define`, then `rustup target add wasm32-unknown-unknown && cargo build --locked --release --lib --target wasm32-unknown-unknown && node tests/top-level-define-wasm.mjs` for native plus real compiled-Wasm verification.

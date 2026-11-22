//! Native Atto std.actor namespace/linking smoke, not host-policy reimplementation.
//! Compiles the actual .atto files through the production Rust compiler.
use std::io;
use std::path::Path;
use atto_compiler::compile_with_imports;

const MAIN: &str = include_str!("../stdlib/actor/guest_smoke.atto");
const POLICY: &str = include_str!("../stdlib/actor/policy.atto");

#[test]
fn std_actor_namespace_and_safe_relative_import_compile_to_guest_wasm() {
    let compiled = compile_with_imports(
        MAIN,
        Path::new("stdlib/actor/guest_smoke.atto"),
        |path| {
            if path == Path::new("stdlib/actor/policy.atto") {
                Ok(POLICY.to_string())
            } else {
                Err(io::Error::new(io::ErrorKind::NotFound, "untrusted actor import"))
            }
        },
    ).expect("real Atto policy with namespace must compile and link");
    assert!(compiled.starts_with(b"\0asm\x01\0\0\0"), "real Wasm artifact required");
}

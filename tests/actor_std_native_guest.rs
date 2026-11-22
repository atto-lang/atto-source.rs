//! Real Rust Atto compiler/linker compilation of native .atto policy guests.
//! Compiling alone does not prove guest execution; Node runs compiled Wasm.
use std::io;
use std::path::Path;
use atto_compiler::compile_with_imports;

const SMOKE: &str = include_str!("../stdlib/actor/guest_smoke.atto");
const EDGES: &str = include_str!("../stdlib/actor/guest_edges.atto");
const NO_IMPORT: &str = include_str!("../stdlib/actor/guest_namespace_only.atto");
const POLICY: &str = include_str!("../stdlib/actor/policy.atto");

fn compile_guest(entry: &str, filename: &str) -> Vec<u8> {
    compile_with_imports(entry, Path::new(filename), |path| {
        if path == Path::new("stdlib/actor/policy.atto") {
            Ok(POLICY.to_string())
        } else {
            Err(io::Error::new(io::ErrorKind::NotFound, "unauthorized actor import"))
        }
    }).expect("production Atto compiler must link the actual std.actor policy")
}

#[test]
fn namespace_actor_smoke_and_boundary_corpus_compile_to_real_wasm() {
    for (entry, name) in [
        (SMOKE, "stdlib/actor/guest_smoke.atto"),
        (EDGES, "stdlib/actor/guest_edges.atto"),
        (NO_IMPORT, "stdlib/actor/guest_namespace_only.atto"),
    ] {
        let wasm = compile_guest(entry, name);
        assert!(wasm.starts_with(b"\0asm\x01\0\0\0"), "real Wasm artifact required");
        assert_eq!(wasm, compile_guest(entry, name), "same source must compile deterministically");
    }
}

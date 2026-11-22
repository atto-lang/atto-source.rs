//! Native Atto source and linked guest checks. No host-language algorithm fallback.
use atto_compiler::linker::{compile_linked, LinkUnit};

const GUEST: &str = include_str!("stdlib/websocket_quic_compute_smoke.atto");
const WS: &str = include_str!("../stdlib/std/websocket.atto");
const QUIC: &str = include_str!("../stdlib/std/quic.atto");
const COMPUTE: &str = include_str!("../stdlib/std/compute.atto");

fn link<'a>(guest: &'a str, ws: &'a str, quic: &'a str, compute: &'a str)
    -> Result<Vec<u8>, atto_compiler::Diagnostic> {
    compile_linked("tests/stdlib/websocket_quic_compute_smoke.atto", &[
        LinkUnit { path: "tests/stdlib/websocket_quic_compute_smoke.atto", package: "atto-app@1", source: guest },
        LinkUnit { path: "stdlib/std/websocket.atto", package: "atto-std@1", source: ws },
        LinkUnit { path: "stdlib/std/quic.atto", package: "atto-std@1", source: quic },
        LinkUnit { path: "stdlib/std/compute.atto", package: "atto-std@1", source: compute },
    ])
}

#[test]
fn actual_module_imports_compile_to_deterministic_guest_wasm() {
    let wasm = link(GUEST, WS, QUIC, COMPUTE).expect("real .atto source-linked guest, no fake stub");
    assert!(wasm.starts_with(b"\0asm\x01\0\0\0"), "compiler did not emit a WebAssembly module");
    assert_eq!(wasm, link(GUEST, WS, QUIC, COMPUTE).expect("repeat compilation"));
}

#[test]
fn missing_import_rejected_before_guest_execution() {
    let missing = GUEST.replace("+stdlib/std/quic.atto", "+stdlib/std/missing.atto");
    let err = link(&missing, WS, QUIC, COMPUTE).expect_err("missing source must not be accepted");
    assert_eq!(err.code, "E_UNBOUND");
}

#[test]
fn private_std_module_not_importable_across_package_boundary() {
    let hidden = WS.replacen("define pub module websocket", "define module websocket", 1);
    let err = link(GUEST, &hidden, QUIC, COMPUTE).expect_err("private module crossed package boundary");
    assert_eq!(err.code, "E_VISIBILITY");
}

#[test]
fn imported_symbol_typo_is_not_an_ambient_global() {
    let typo = GUEST.replacen("quic.quic_encoding_min_width", "quic.quic_min_width_bogus", 1);
    let err = link(&typo, WS, QUIC, COMPUTE).expect_err("missing member must fail");
    assert!(matches!(err.code, "E_UNBOUND" | "E_UNKNOWN_NAME" | "E_MEMBER" | "E_CALL"), "unexpected diagnostic: {err}");
}

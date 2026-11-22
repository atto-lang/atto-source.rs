//! Build real std/websocket, std/quic, std/compute Atto guest to a Wasm file.
//! This runner does NOT implement protocol rules in Rust; they are .atto.
use atto_compiler::linker::{compile_linked, LinkUnit};

fn main() {
    let mut args = std::env::args().skip(1);
    let path = args.next().expect("expected output.wasm path");
    assert!(args.next().is_none(), "one output path only");
    let guest = compile_linked("tests/stdlib/websocket_quic_compute_smoke.atto", &[
        LinkUnit {
            path: "tests/stdlib/websocket_quic_compute_smoke.atto",
            package: "atto-app@1",
            source: include_str!("../tests/stdlib/websocket_quic_compute_smoke.atto"),
        },
        LinkUnit {
            path: "stdlib/std/websocket.atto",
            package: "atto-std@1",
            source: include_str!("../stdlib/std/websocket.atto"),
        },
        LinkUnit {
            path: "stdlib/std/quic.atto",
            package: "atto-std@1",
            source: include_str!("../stdlib/std/quic.atto"),
        },
        LinkUnit {
            path: "stdlib/std/compute.atto",
            package: "atto-std@1",
            source: include_str!("../stdlib/std/compute.atto"),
        },
    ]).expect("compile actual source-linked Atto std modules");
    assert!(guest.starts_with(b"\0asm\x01\0\0\0"));
    std::fs::write(path, guest).expect("write compiled Wasm");
}

//! Build actual Atto-linked std libraries into independently executed Wasm guests.
//! Never substitute Rust host models for native .atto implementation.
use atto_compiler::linker::{compile_linked, LinkUnit};

fn build(source: &str) -> Vec<u8> {
    compile_linked("tests/stdlib/websocket_quic_compute_smoke.atto", &[
        LinkUnit {
            path: "tests/stdlib/websocket_quic_compute_smoke.atto",
            package: "atto-app@1",
            source,
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
    ]).expect("compile actual source-linked Atto std modules")
}

fn main() {
    let mut args = std::env::args().skip(1);
    let success_path = args.next().expect("expected success.wasm path");
    let mutation_path = args.next().expect("expected mutation.wasm path");
    let boundary_path = args.next().expect("expected boundaries.wasm path");
    let borrow_path = args.next().expect("expected borrow_core.wasm path");
    assert!(args.next().is_none(), "expected exactly four output paths");

    const ORIGINAL: &str = "quic.quic_varint_octet(15293, 2, 0) != 123";
    const MUTATED: &str = "quic.quic_varint_octet(15293, 2, 0) != 124";
    let source = include_str!("../tests/stdlib/websocket_quic_compute_smoke.atto");
    assert_eq!(source.matches(ORIGINAL).count(), 1, "exactly one mutation target");
    let failure_source = source.replacen(ORIGINAL, MUTATED, 1);

    let success = build(source);
    let failure = build(&failure_source);
    let boundaries = build(include_str!("../tests/stdlib/protocol_boundaries.atto"));
    let borrow_core = build(include_str!("../tests/stdlib/protocol_borrow_core.atto"));
    assert!(success.starts_with(b"\0asm\x01\0\0\0"));
    assert!(failure.starts_with(b"\0asm\x01\0\0\0"));
    assert!(boundaries.starts_with(b"\0asm\x01\0\0\0"));
    assert!(borrow_core.starts_with(b"\0asm\x01\0\0\0"));
    assert_ne!(success, failure, "mutated Atto source must change compiled guest bytes");
    std::fs::write(success_path, success).expect("write normal compiled guest");
    std::fs::write(mutation_path, failure).expect("write intentionally failing compiled guest");
    std::fs::write(boundary_path, boundaries).expect("write independent compiled boundary guest");
    std::fs::write(borrow_path, borrow_core).expect("write source-compiled ownership guest");
}

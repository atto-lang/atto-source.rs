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

#[test]
fn unsupported_host_remainder_operator_never_emits_a_guest() {
    // Rust/Wasm explicitly does not lower '%' in this compiler subset.
    // A source change cannot silently inherit a different backend's feature.
    const VALID: &str = "(a - (a / 2) * 2)";
    assert!(WS.contains(VALID), "positive fixture must use supported scalar math");
    let unsupported = WS.replacen(VALID, "a % 2", 1);
    let err = link(GUEST, &unsupported, QUIC, COMPUTE)
        .expect_err("unsupported remainder operator must not produce Wasm");
    assert_eq!(err.code, "E_UNSUPPORTED");
}

#[test]
fn unqualified_internal_module_call_does_not_gain_ambient_authority() {
    const QUALIFIED: &str = "websocket.websocket_valid_opcode(";
    assert!(WS.contains(QUALIFIED), "expected explicit module-owned helper call");
    let unqualified = WS.replacen(QUALIFIED, "websocket_valid_opcode(", 1);
    let err = link(GUEST, &unqualified, QUIC, COMPUTE)
        .expect_err("unqualified same-module helper must not be ambient");
    assert!(matches!(err.code, "E_UNBOUND" | "E_UNKNOWN_NAME" | "E_TYPE" | "E_CALL"),
        "unexpected unqualified call rejection: {err}");
}

const BOUNDARIES: &str = include_str!("stdlib/protocol_boundaries.atto");

#[test]
fn independently_linked_109_case_boundary_guest_emits_wasm_deterministically() {
    let bytes = link(BOUNDARIES, WS, QUIC, COMPUTE)
        .expect("compile independent real .atto boundary guest");
    assert!(bytes.starts_with(b"\0asm\x01\0\0\0"));
    assert_eq!(bytes, link(BOUNDARIES, WS, QUIC, COMPUTE).unwrap(),
        "source-linked Wasm output must be deterministic");
}

#[test]
fn boundary_guest_cannot_import_an_undeclared_private_module() {
    let unlisted = BOUNDARIES.replace(
        "import module compute from \"+stdlib/std/compute.atto\";",
        "",
    );
    let err = link(&unlisted, WS, QUIC, COMPUTE)
        .expect_err("unreachable imports or ambient module calls cannot create guest Wasm");
    assert!(matches!(err.code, "E_UNBOUND" | "E_VISIBILITY" | "E_UNSUPPORTED"),
        "unexpected undeclared-import diagnostic: {err}");
}

#[test]
fn boundary_guest_exposes_every_public_std_helper() {
    let modules: [(&str, &str); 3] = [
        ("websocket", WS), ("quic", QUIC), ("compute", COMPUTE),
    ];
    for (name, module) in modules {
        for part in module.split("pub fnc ").skip(1) {
            let function = part.split('(').next().expect("function name").trim();
            assert!(!function.is_empty(), "empty exported function");
            assert!(BOUNDARIES.contains(&format!("{name}.{function}(")),
                "independent Atto boundary guest does not exercise {name}.{function}");
        }
    }
}


const BORROW_CORE: &str = include_str!("stdlib/protocol_borrow_core.atto");

fn borrow_core_mutation(before: &str, after: &str, expected: &str) {
    assert_eq!(BORROW_CORE.matches(before).count(), 1, "mutation anchor must be unique");
    let mutant = BORROW_CORE.replacen(before, after, 1);
    assert_ne!(mutant, BORROW_CORE, "negative test must modify actual Atto source");
    let err = link(&mutant, WS, QUIC, COMPUTE)
        .expect_err("unsafe borrowed ownership must not emit executable guest Wasm");
    assert_eq!(err.code, expected, "unexpected compiler diagnostic: {err}");
}

#[test]
fn scoped_core_borrows_can_call_real_protocol_atto_modules_and_move_after_end() {
    let wasm = link(BORROW_CORE, WS, QUIC, COMPUTE)
        .expect("positive borrowed buffer descriptor must compile through the real core checker");
    assert!(wasm.starts_with(b"\0asm\x01\0\0\0"), "must emit a true Wasm guest");
    assert_eq!(wasm, link(BORROW_CORE, WS, QUIC, COMPUTE).unwrap(),
        "ownership-checking decisions must remain deterministic");
}

#[test]
fn core_checker_rejects_owner_move_with_live_read_loan() {
    borrow_core_mutation(
        "val reader = borrow! buffer;",
        "val reader = borrow! buffer;\n    val premature = take! buffer;",
        "E_BORROW");
}

#[test]
fn core_checker_rejects_mutation_through_shared_read_loan() {
    borrow_core_mutation(
        "val reader = borrow! buffer;",
        "val reader = borrow! buffer;\n    reader.consumed = 9;",
        "E_BORROW");
}

#[test]
fn core_checker_rejects_conflicting_loan_while_writer_is_live() {
    borrow_core_mutation(
        "val writer = borrow! mut buffer;",
        "val writer = borrow! mut buffer;\n    val conflicted = borrow! buffer;",
        "E_BORROW");
}

#[test]
fn core_checker_rejects_reading_owner_during_exclusive_loan() {
    borrow_core_mutation(
        "val writer = borrow! mut buffer;",
        "val writer = borrow! mut buffer;\n    val leaked = buffer.capacity;",
        "E_BORROW");
}

#[test]
fn core_checker_rejects_use_after_take_even_when_protocol_call_is_legal() {
    borrow_core_mutation(
        "val transferred = take! buffer;",
        "val transferred = take! buffer;\n  val stale = buffer.capacity;",
        "E_MOVED");
}

#[test]
fn core_checker_does_not_borrow_an_unowned_temporary_descriptor() {
    borrow_core_mutation(
        "val reader = borrow! buffer;",
        "val reader = borrow! new BoundedBuffer();",
        "E_BORROW");
}


#[test]
fn core_checker_rejects_exclusive_loan_while_two_shared_loans_are_live() {
    borrow_core_mutation(
        "val second_reader = borrow! buffer;",
        "val second_reader = borrow! buffer;\n    val writer = borrow! mut buffer;",
        "E_BORROW");
}

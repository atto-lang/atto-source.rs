//! Real Rust/Wasm frontend compilation gates for canonical copy!! syntax.
//! Identity-free scalars and verified FLAT class copies compile to Wasm.
//! Nested mutable graph, Sendable, inheritance and actor-heap copies remain
//! fail closed until distinct proof and code-generation machinery exists.
use atto_compiler::compile;

fn wasm(source: &str) -> Vec<u8> {
    let bytes = compile(source).unwrap_or_else(|d| panic!("{source}: {d}"));
    assert_eq!(&bytes[..8], b"\0asm\x01\0\0\0");
    assert_eq!(compile(source).unwrap(), bytes, "deterministic source lowering");
    bytes
}

#[test]
fn scalar_deep_copy_produces_real_wasm_bytes() {
    wasm("pub fnc main(): int { val x = 41; val y = copy!! x; return y + 1; }");
    wasm("pub fnc main(): int { return copy!!(42); }");
}

#[test]
fn copy_bang_then_space_bang_is_not_the_deep_operator() {
    // The contiguous double-bang spelling is the deep operation; this
    // spaced spelling must remain shallow-copy of a logical negation.
    wasm("pub fnc main(): int { val flag = copy! !false; return flag ? 42 : 0; }");
}

#[test]
fn generated_flat_class_copier_emits_real_wasm_bytes() {
    wasm(include_str!("fixtures/pass/993-copy-doublebang-flat-class.atto"));
    wasm(include_str!("fixtures/pass/994-copy-doublebang-two-independent-class-copies.atto"));
    wasm(include_str!("fixtures/pass/995-copy-doublebang-flat-class-mixed-fields.atto"));
    wasm(include_str!("fixtures/pass/996-copy-doublebang-constructor-state.atto"));
    wasm(include_str!("fixtures/pass/997-copy-doublebang-private-state.atto"));
    wasm(include_str!("fixtures/pass/998-copy-doublebang-pod-mixed-fields.atto"));
    wasm(include_str!("fixtures/pass/999-copy-doublebang-pod-interleaved-snapshots.atto"));
    wasm("define class Parcel as pub let int value = 1; end pub fnc main(): int { val q = copy!! (new Parcel()); return q.value; }");
}

#[test]
fn an_unverified_custom_copy_deep_method_fails_closed() {
    let src = include_str!("fixtures/fail/991-copy-doublebang-class-unproved.atto");
    let diagnostic = compile(src).expect_err("user copy_deep override is not yet verified");
    assert_eq!(diagnostic.code, "E_COPY_OVERRIDE", "{diagnostic}");
}

#[test]
fn copied_class_cannot_be_read_during_exclusive_mutable_loan() {
    let src = include_str!("fixtures/fail/996-copy-doublebang-exclusive-loan.atto");
    let diagnostic = compile(src).expect_err("cannot read an exclusive borrowed owner");
    assert_eq!(diagnostic.code, "E_BORROW", "{diagnostic}");
}

#[test]
fn nested_array_copy_fails_closed_before_codegen() {
    let source = include_str!("fixtures/fail/997-copy-doublebang-nested-array-no-runtime.atto");
    let diagnostic = compile(source).expect_err("Array<Array<int>> requires a checked allocation/index ABI");
    assert_eq!(diagnostic.code, "E_UNSUPPORTED_TYPE", "{diagnostic}");
}

#[test]
fn copy_deep_disallows_mut_qualifier_and_missing_operand() {
    let diagnostic = compile("pub fnc main(): int { val x = 1; return copy!! mut x; }").expect_err("mut qualifier is reserved for borrow!");
    assert_eq!(diagnostic.code, "E_SYNTAX");
    assert!(compile("pub fnc main(): int { return copy!!; }").is_err());
}

#[test]
fn separate_bangs_and_spelling_have_distinct_parsed_behavior() {
    // A shallow class copy without verified Copyable witness must still
    // fail. The deep path is independently admitted for this flat class.
    let prefix = "define class Parcel as pub let int value = 1; end pub fnc main(): int { val p = new Parcel(); ";
    let shallow = format!("{prefix}val q = copy! p; return 0; }}");
    assert_eq!(compile(&shallow).unwrap_err().code, "E_COPY");
    let deep = format!("{prefix}val q = copy!! p; return q.value; }}");
    wasm(&deep);
}

#[test]
fn verified_pointer_free_class_uses_a_single_bulk_memory_copy() {
    // The compiler emits the core Wasm memory.copy instruction for the
    // already-admitted POD class rather than generating per-field loads.
    // The existing guest-execution harness also verifies copied-object
    // independence after source/copy mutation.
    let bytes = wasm(include_str!("fixtures/pass/998-copy-doublebang-pod-mixed-fields.atto"));
    assert!(bytes.windows(4).any(|b| b == [0xFC, 0x0A, 0, 0]),
        "flat POD copier should contain a Wasm memory.copy instruction");
}

#[test]
fn immutable_string_backing_does_not_take_the_pod_bulk_copy_path() {
    // String fields are explicitly excluded from the raw-byte fast path;
    // even if their present ABI is packed i64, they are reference-bearing.
    let bytes = wasm(include_str!("fixtures/pass/995-copy-doublebang-flat-class-mixed-fields.atto"));
    assert!(!bytes.windows(4).any(|b| b == [0xFC, 0x0A, 0, 0]),
        "string-bearing class must retain typed field-by-field cloning");
}

#[test]
fn generated_pod_copy_uses_only_checked_local_slots() {
    use atto_compiler::{check::{self, TStmt}, lexer, parser::Parser};
    let source = include_str!("fixtures/pass/998-copy-doublebang-pod-mixed-fields.atto");
    let ast = Parser::new(lexer::lex(source).unwrap()).parse_program().unwrap();
    let typed = check::check(&ast).unwrap();
    let copier = typed.funcs.iter()
        .find(|f| f.name == "Snapshot.copy_deep$generated")
        .expect("concrete generated copier");
    assert!(matches!(copier.body.get(1),
        Some(TStmt::FlatPodCopy { dest_slot: 1, source_slot: 0, bytes: 24 })),
        "verified POD plan must copy exact byte size from source parameter to fresh destination");
    assert_eq!(copier.body.iter().filter(|s|
        matches!(s, TStmt::FlatPodCopy { .. })).count(), 1);
}

#[test]
fn string_class_never_admits_raw_byte_copy_ir() {
    use atto_compiler::{check::{self, TStmt}, lexer, parser::Parser};
    let source = include_str!("fixtures/pass/995-copy-doublebang-flat-class-mixed-fields.atto");
    let ast = Parser::new(lexer::lex(source).unwrap()).parse_program().unwrap();
    let typed = check::check(&ast).unwrap();
    let copier = typed.funcs.iter()
        .find(|f| f.name == "Parcel.copy_deep$generated")
        .expect("concrete generated copier");
    assert!(!copier.body.iter().any(|s| matches!(s, TStmt::FlatPodCopy { .. })),
        "string-bearing class must never lower to raw memory copying");
    assert_eq!(copier.body.iter().filter(|s|
        matches!(s, TStmt::FieldSet { .. })).count(), 3);
}

#[test]
fn wasm_backend_rejects_forged_or_corrupt_flat_copy_ir() {
    use atto_compiler::{check::{self, TStmt}, lexer, parser::Parser, wasm as codegen};
    use std::panic::{catch_unwind, AssertUnwindSafe};
    let source = include_str!("fixtures/pass/998-copy-doublebang-pod-mixed-fields.atto");
    let ast = Parser::new(lexer::lex(source).unwrap()).parse_program().unwrap();
    let trusted = check::check(&ast).unwrap();
    let copier_index = trusted.funcs.iter()
        .position(|f| f.name == "Snapshot.copy_deep$generated")
        .expect("generated private class copier");
    assert!(!codegen::emit(&trusted).is_empty(), "canonical copy IR must emit");
    // A public TStmt representation must not let a future lowerer bypass
    // AOT copy-plan admission by writing a plausible-looking instruction.
    for corruption in 0..7 {
        let mut forged = trusted.clone();
        let copier = &mut forged.funcs[copier_index];
        match corruption {
            0 => {
                if let TStmt::FlatPodCopy { source_slot, .. } = &mut copier.body[1] {
                    *source_slot = 2;
                }
            }
            1 => {
                if let TStmt::FlatPodCopy { dest_slot, .. } = &mut copier.body[1] {
                    *dest_slot = 0;
                }
            }
            2 => {
                if let TStmt::FlatPodCopy { bytes, .. } = &mut copier.body[1] {
                    *bytes = 16; // size mismatch: actual allocation is 24
                }
            }
            3 => { copier.name = "forged_copy".to_owned(); }
            4 => { copier.locals.clear(); } // invalid destination type/slot
            5 => { copier.body.push(TStmt::Unreachable); } // noncanonical copier
            6 => {
                // A private raw-memory opcode inserted into an arbitrary
                // function must fail regardless of surrounding control flow.
                forged.funcs[forged.main as usize].body.push(
                    TStmt::FlatPodCopy { dest_slot: 1, source_slot: 0, bytes: 24 });
            }
            _ => unreachable!(),
        }
        let did_panic = catch_unwind(AssertUnwindSafe(|| codegen::emit(&forged))).is_err();
        assert!(did_panic, "unverified FlatPodCopy mutation {corruption} emitted Wasm");
    }
}

//! Production parser/semantic type contracts, independent of the bounded
//! source-compiled guest specialization. Generic type support alone MUST NOT
//! admit unproved general-purpose allocation, borrowing or transport.

use atto_compiler::{ast::{Stmt, Type}, check, lexer, parser::Parser};

fn parse(source: &str) -> atto_compiler::ast::Program {
    Parser::new(lexer::lex(source).expect("lex")).parse_program().expect("main parser")
}

#[test]
fn canonical_reordered_named_generics_share_one_type_identity() {
    let p = parse(r#"
        fnc main() -> int {
            let first: Array<Particle, layout=soa> = [];
            let second: Array<layout=soa, T=Particle> = [];
            let third: Array<T=Particle, layout=soa> = [];
            return 0;
        }
    "#);
    assert_eq!(p.array_types.len(), 1, "reordering named args must not change type identity");
    assert_eq!(p.array_types[0].element, "Particle");
    assert_eq!(p.array_types[0].layout, atto_compiler::collection_layout_plan::Storage::Soa);
    let f = &p.funcs[0];
    for stmt in &f.body[..3] {
        let Stmt::Bind { ty: Some(Type::Array(i)), .. } = stmt else { panic!("missing typed Array") };
        assert_eq!(*i, 0);
    }
}

#[test]
fn default_aos_is_canonical_and_soa_aosoa_remain_distinct() {
    let p = parse(r#"
        fnc main() -> int {
            let x: Array<Particle> = [];
            let y: Array<T=Particle, layout=aos> = [];
            let z: Array<Particle, layout=soa> = [];
            let w: Array<Particle, layout=aosoa(16)> = [];
            return 0;
        }
    "#);
    assert_eq!(p.array_types.len(), 3);
    let slots: Vec<u32> = p.funcs[0].body.iter().filter_map(|stmt| match stmt {
        Stmt::Bind { ty: Some(Type::Array(i)), .. } => Some(*i),
        _ => None,
    }).collect();
    assert_eq!(slots, [0, 0, 1, 2]);
}

#[test]
fn nominal_element_is_checked_and_runtime_requires_proven_lowering() {
    let missing = parse("fnc main() -> int { let a: Array<Undefined,layout=soa> = []; return 0; }");
    assert_eq!(check::check(&missing).unwrap_err().code, "E_UNKNOWN_TYPE");
    let declared = parse(r#"
        define class Particle as end
        fnc main() -> int {
            let a: Array<Particle, layout=soa> = [];
            return 0;
        }
    "#);
    assert_eq!(declared.array_types.len(), 1);
    let err = check::check(&declared).expect_err("general Array allocation requires proven storage and borrow checking");
    assert_eq!(err.code, "E_UNSUPPORTED");
    assert!(err.message.contains("general-purpose Array storage"));
}

#[test]
fn generic_errors_from_production_parser_match_guest_parser() {
    for (generic, expected) in [
        ("Array<Particle, layout=bogus>", "E_LAYOUT_VALUE"),
        ("Array<Particle, layout=aosoa(n)>", "E_LAYOUT_SIZE"),
        ("Array<layout=soa, Particle>", "E_GENERIC_ORDER"),
        ("Array<Particle, layout=soa, layout=aos>", "E_GENERIC_DUP"),
        ("Array<Particle, bogus=soa>", "E_GENERIC_NAME"),
    ] {
        let source = format!("fnc main() -> int {{ let x: {generic} = []; return 0; }}");
        let err = Parser::new(lexer::lex(&source).unwrap()).parse_program().unwrap_err();
        assert_eq!(err.code, expected, "{generic}");
    }
}

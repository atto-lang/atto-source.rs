//! Parser target only; positive source must still fail closed before guest emission.
use atto_compiler::{compile, lexer::lex, parser::Parser, ast::Type};

const GROUPED: &str = r#"
type Point = struct {
    int x;
    int y;
} with {
    constructor(int x, int y) { self.x = x; self.y = y; }
    mtd int • sum() { return self.x + self.y; }
    mtd int <- product() { return self.x * self.y; }
    mtd is_origin() : bool { return self.x == 0 && self.y == 0; }
}
"#;
const CHAINED: &str = r#"
type Point = struct { int x; int y; }
with constructor(int x, int y) { self.x = x; self.y = y; }
with mtd int • sum() { return self.x + self.y; }
with mtd int <- product() { return self.x * self.y; }
with mtd is_origin() : bool { return self.x == 0 && self.y == 0; }
"#;

#[test]
fn grouped_chained_and_return_forms_share_signature() {
    for source in [GROUPED.to_owned(), CHAINED.to_owned(),
        GROUPED.replace("int • sum()", "sum(): int"),
        GROUPED.replace("int <- product()", "int • product()"),
        GROUPED.replace("is_origin() : bool", "bool <- is_origin()")] {
        let d = Parser::new(lex(&source).unwrap()).parse_struct_with_target().unwrap();
        assert_eq!(d.name, "Point");
        assert_eq!(d.fields, vec![("x".to_string(), Type::Int), ("y".to_string(), Type::Int)]);
        assert_eq!(d.constructors.len(), 1);
        assert_eq!(d.constructors[0].1.params.len(), 2);
        assert_eq!(d.methods.iter().map(|m| (m.name.as_str(), m.ret)).collect::<Vec<_>>(),
            vec![("sum", Type::Int), ("product", Type::Int), ("is_origin", Type::Bool)]);
    }
}

#[test]
fn invalid_headers_and_duplicates_reject() {
    for source in [
        GROUPED.replace("is_origin() : bool", "bool * is_origin()"),
        GROUPED.replace("int <- product()", "int <- sum()"),
        GROUPED.replace("int y;", "int x;"),
        GROUPED.replace("constructor(int x, int y)", "print(42); constructor(int x, int y)"),
        GROUPED.replace("} with {", "} with { } with {"),
    ] {
        assert!(Parser::new(lex(&source).unwrap()).parse_struct_with_target().is_err());
    }
}

#[test]
fn constructor_overload_headers_are_distinct() {
    let src = GROUPED.replace("constructor(int x, int y)",
        "constructor() { self.x = 0; self.y = 0; } constructor(int x, int y)");
    let parsed = Parser::new(lex(&src).unwrap()).parse_struct_with_target().unwrap();
    assert_eq!(parsed.constructors.len(), 2);
}

#[test]
fn no_struct_class_fallback_or_unverified_guest() {
    let source = format!("{GROUPED}\npub fnc main(): int {{ return 0; }}");
    let error = compile(&source).expect_err("target-only struct cannot emit a guest artifact");
    assert_eq!(error.code, "E_UNSUPPORTED");
    let invalid = source.replace("is_origin() : bool", "bool * is_origin()");
    assert!(compile(&invalid).is_err());
}

#[test]
fn scalar_constructor_initialization_rejects_missing_duplicate_unknown_wrong_type_and_early_read() {
    for src in [
        GROUPED.replace("self.y = y;", ""),
        GROUPED.replace("self.y = y;", "self.x = y;"),
        GROUPED.replace("self.y = y;", "self.z = y;"),
        GROUPED.replace("self.y = y;", "self.y = true;"),
        GROUPED.replace("self.x = x;", "self.x = self.y;"),
        GROUPED.replace("self.x = x;", "self.x = publish(self);"),
    ] {
        let err = Parser::new(lex(&src).unwrap()).parse_struct_with_target()
            .expect_err("invalid constructor must fail closed");
        assert_eq!(err.code, "E_STRUCT_INIT");
    }
}

#[test]
fn constructor_can_read_an_earlier_initialized_scalar() {
    let src = GROUPED.replace("self.y = y;", "self.y = self.x + y;");
    assert!(Parser::new(lex(&src).unwrap()).parse_struct_with_target().is_ok());
}

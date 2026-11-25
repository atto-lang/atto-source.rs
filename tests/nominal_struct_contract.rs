//! Restricted, nominal source-local Struct profile (not an unboxed value ABI).
//! All admitted guests use the existing checked class-backed allocation.
//! These tests protect the portable distinction from structural Records.

use atto_compiler::compile;

fn admitted(source: &str) {
    let bytes = compile(source).unwrap_or_else(|err| panic!("{source}\n{err}"));
    assert!(bytes.starts_with(b"\0asm\x01\0\0\0"));
    assert_eq!(bytes, compile(source).unwrap(), "deterministic guest lowering");
}

fn rejected(source: &str, code: &str) {
    let err = compile(source).expect_err("invalid nominal Struct must not emit Wasm");
    assert_eq!(err.code, code, "{source}\n{err}");
}

#[test]
fn declared_structs_have_named_initializers_and_real_guest_field_reads() {
    admitted(
        r#"define struct Point as
        int x;
        int y;
    end
    pub fnc main(): int {
        val Point p = Point{x:40,y:2};
        return p.x+p.y;
    }"#,
    );
    admitted(
        r#"define struct Point as
        int x;
        int y;
    end
    pub fnc main(): int {
        val Point p = Point{y:2,x:40};
        return p.x+p.y;
    }"#,
    );
}

#[test]
fn same_schema_different_names_are_distinct_nominal_types() {
    admitted(
        r#"define struct Point as int x; int y; end
    define struct Position as int x; int y; end
    pub fnc main(): int {
        val Point p=Point{x:40,y:2};
        val Position q=Position{x:40,y:2};
        return p.x+q.y;
    }"#,
    );
    rejected(
        r#"define struct Point as int x; int y; end
    define struct Position as int x; int y; end
    pub fnc main(): int {
        val Position q=Position{x:40,y:2};
        val Point p=q;
        return 0;
    }"#,
        "E_TYPE",
    );
}

#[test]
fn exact_struct_schema_and_identifier_field_names_are_required() {
    rejected(
        r#"define struct Point as int x; int y; end
    pub fnc main(): int {val Point p=Point{x:42};return 0;}"#,
        "E_SHAPE",
    );
    rejected(
        r#"define struct Point as int x; int y; end
    pub fnc main(): int {val Point p=Point{x:40,y:2,z:true};return 0;}"#,
        "E_SHAPE",
    );
    rejected(
        r#"define struct Point as int x; int y; end
    pub fnc main(): int {val Point p=Point{x:40,x:41,y:2};return 0;}"#,
        "E_DUPLICATE",
    );
    rejected(
        r#"define struct Point as int "x-y"; int y; end
    pub fnc main(): int {return 0;}"#,
        "E_SYNTAX",
    );
}

#[test]
fn a_struct_is_not_an_implicit_record_cast() {
    rejected(
        r#"define struct Point as int x; int y; end
    pub fnc main(): int {
        val Record r=(x=40,y=2);
        val Point p=r;
        return 0;
    }"#,
        "E_TYPE",
    );
}

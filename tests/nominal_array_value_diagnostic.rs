use atto_compiler::{compile, compile_js};

#[test]
fn nominal_type_is_not_array_data() {
    let source = include_str!("fixtures/fail/999-class-type-as-list-element.atto");
    assert_eq!(compile(source).expect_err("nominal type is not a value").code, "E_TYPE_NOT_VALUE");
    assert_eq!(compile_js(source).expect_err("nominal type is not a value").code, "E_TYPE_NOT_VALUE");
}

#[test]
fn array_backend_remains_fail_closed() {
    for source in [
        "pub fnc main(): int { val values = [42]; return 0; }",
        "pub fnc main(): int { val x = 42; val values = [x]; return 0; }",
        "define class Parcel as end pub fnc main(): int { val values = [new Parcel()]; return 0; }",
    ] {
        assert_eq!(compile(source).expect_err("no ordinary array lowering").code, "E_UNSUPPORTED", "{source}");
        assert_eq!(compile_js(source).expect_err("no ordinary array lowering").code, "E_UNSUPPORTED", "{source}");
    }
}

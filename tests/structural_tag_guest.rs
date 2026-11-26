//! Production compile() coverage for source-local static brand admission.
//! The separate Node guest runner executes compiler-generated Wasm.
use atto_compiler::compile;

const NOMINAL: &str = include_str!("markers-brands-v1/same-brand-nominal.atto");
const CROSS: &str = include_str!("markers-brands-v1/same-brand-cross-nominal.atto");
const WRONG: &str = include_str!("markers-brands-v1/reject-wrong-brand.atto");
const PLAIN: &str = include_str!("markers-brands-v1/reject-unbranded.atto");

#[test]
fn nominal_and_cross_nominal_same_brand_emit_wasm() {
    for source in [NOMINAL, CROSS] {
        let wasm = compile(source).expect("same brand must compile");
        assert_eq!(&wasm[..8], b"\0asm\x01\0\0\0");
        assert_eq!(wasm, compile(source).unwrap());
    }
}

#[test]
fn incompatible_brand_not_just_unsupported_syntax() {
    for source in [WRONG, PLAIN] {
        assert_eq!(compile(source).unwrap_err().code, "E_MISSING_BRAND");
    }
}

#[test]
fn long_and_short_structural_spelling_and_tag_erasure() {
    let long = CROSS.replace("~UserId", "structural UserId");
    let expected = compile(CROSS).unwrap();
    assert_eq!(expected, compile(&long).unwrap());
    let untagged = CROSS.lines()
        .filter(|line| !line.starts_with("define pub brand "))
        .collect::<Vec<_>>().join("\n")
        .replace(" with brand #user_id", "")
        .replace(" with brand #order_id", "");
    assert_eq!(expected, compile(&untagged).unwrap(),
        "static domain tags must not add payload bytes or runtime tag checks");
}

#[test]
fn named_marker_constraints_are_conjunctive() {
    let source = r#"
define pub marker #entity;
define pub marker #auditable;
define class Required with marker #entity, #auditable as
    pub int value = 42;
end
define class Actual with marker #entity, #auditable as
    pub int value = 42;
end
fnc read(~Required p): int { return p.value; }
pub fnc main(): int { val v = new Actual(); return read(v); }
"#;
    assert!(compile(source).is_ok());
    let missing = source.replace("Actual with marker #entity, #auditable", "Actual with marker #entity");
    assert_eq!(compile(&missing).unwrap_err().code, "E_MISSING_MARKER");
}

#[test]
fn brand_is_not_a_field_witness() {
    let source = CROSS.replace(
        "OtherUserId with brand #user_id as\n    pub int value = 42;",
        "OtherUserId with brand #user_id as\n    pub int other = 42;");
    assert_ne!(source, CROSS);
    assert_eq!(compile(&source).unwrap_err().code, "E_TYPE");
}

#[test]
fn invalid_tag_declarations_are_rejected() {
    for source in [
        CROSS.replace("define pub brand #order_id;", "define pub brand #user_id;"),
        CROSS.replace("OtherUserId with brand #user_id", "OtherUserId with brand #unknown"),
        CROSS.replace("OtherUserId with brand #user_id", "OtherUserId with marker #user_id"),
        CROSS.replace("OtherUserId with brand #user_id", "OtherUserId with brand #user_id, #user_id"),
    ] {
        assert_eq!(compile(&source).unwrap_err().code, "E_TAG_DECLARATION");
    }
}

#[test]
fn unsupported_tagged_patterns_and_unchecked_call_paths_fail_closed() {
    for source in [
        CROSS.replace("return read_user(id);", "return read_user(new OtherUserId());"),
        CROSS.replace("return read_user(id);", "return 42;"),
        CROSS.replace("return read_user(id);", "return #user_id;"),
        CROSS.replace("return id.value;", "return id;"),
        CROSS.replace("val id = ", "let id = "),
    ] {
        assert_eq!(compile(&source).unwrap_err().code, "E_TAG_UNSUPPORTED");
    }
    assert!(compile("pub fnc main(): int { return #user_id; }").is_err());
}

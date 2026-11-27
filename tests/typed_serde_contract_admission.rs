//! Typed serialization contract admission, not codec implementation.
//! Existing nominal compiler checks must not synthesize or silently erase
//! serialization contracts before a verified typed serializer ABI exists.
use atto_compiler::compile;

fn reject(src: &str, code: &str) {
    let d = compile(src).expect_err("unproved serialization contract emitted guest Wasm");
    assert_eq!(d.code, code, "{d}");
    assert!(d.pos.line > 0 && d.pos.col > 0, "{d}");
}

#[test]
fn struct_does_not_gain_json_capability_from_interface_name() {
    reject(r#"
        define interface JsonSerializable as to_json(): string; end
        define struct User implements JsonSerializable as value: int; end
        pub fnc main(): int { return 0; }
    "#, "E_IMPL_MISSING");
}

#[test]
fn actor_cannot_satisfy_json_contract_with_wrong_return() {
    reject(r#"
        define interface JsonSerializable as to_json(): string; end
        define actor A implements JsonSerializable as
            pub to_json(): int { return 42; }
        end
        pub fnc main(): int { return 0; }
    "#, "E_IMPL_SIGNATURE");
}

#[test]
fn abstract_deserialization_trait_requires_concrete_method() {
    reject(r#"
        define trait JsonDeserializable as from_json(string input): string; end
        define struct User implements JsonDeserializable as value: int; end
        pub fnc main(): int { return 0; }
    "#, "E_IMPL_MISSING");
}

#[test]
fn fallible_serialization_signature_cannot_be_erased_into_scalar_abi() {
    reject(r#"
        define interface JsonSerializable as to_json(): Result<string, EncodeError>; end
        define struct User implements JsonSerializable as value: int; end
        pub fnc main(): int { return 0; }
    "#, "E_UNSUPPORTED");
}

#[test]
fn ordinary_codec_named_function_is_not_a_reserved_keyword() {
    let src = r#"
        pub fnc to_json(): int { return 42; }
        pub fnc main(): int { return to_json(); }
    "#;
    assert!(compile(src).unwrap().starts_with(b"\0asm\x01\0\0\0"));
}

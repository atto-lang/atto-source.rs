//! File-level export declarations are order-independent and checked by kind.
use atto_compiler::visibility_lower::Visibility;
use atto_compiler::{compile, lexer, parser};

fn parse(source: &str) -> atto_compiler::ast::Program {
    parser::Parser::new(lexer::lex(source).expect("lex")).parse_program().expect("parse")
}

const DECLS: &str = r#"
define class Foo as
end
define class Bar as
end
fnc calculate(int n): int { return n + 1; }
pub fnc main(): int { return calculate(41); }
"#;

const EXPORTS: &str = r#"
export pub Foo;
export pub[pkg] class Bar;
export pub fnc calculate;
"#;

#[test]
fn both_export_spellings_and_package_visibility_parse() {
    let program = parse(&format!("{EXPORTS}{DECLS}"));
    assert_eq!(program.parsed_exports.len(), 3);
    assert_eq!(program.parsed_exports[0].name, "Foo");
    assert_eq!(program.parsed_exports[0].kind, "");
    assert_eq!(program.parsed_exports[0].visibility, Visibility::Public);
    assert_eq!(program.parsed_exports[1].name, "Bar");
    assert_eq!(program.parsed_exports[1].kind, "class");
    assert_eq!(program.parsed_exports[1].visibility, Visibility::Package);
    assert_eq!(program.parsed_exports[2].kind, "fnc");
}

#[test]
fn moving_exports_from_top_to_bottom_preserves_binary_and_metadata() {
    let top = compile(&format!("{EXPORTS}{DECLS}")).expect("top exports");
    let bottom = compile(&format!("{DECLS}{EXPORTS}")).expect("bottom exports");
    assert_eq!(top, bottom, "export resolution must be source-order independent");
    assert!(top.windows(b"atto.exports.v1".len()).any(|w| w == b"atto.exports.v1"));
    assert!(top.windows(b"calculate".len()).any(|w| w == b"calculate"));
}

#[test]
fn explicit_and_inferred_export_kinds_agree() {
    let src = format!(
        "export pub class Foo;\nexport pub Bar;\nexport pub[pkg] fnc calculate;\n{DECLS}"
    );
    assert!(compile(&src).is_ok());
}

#[test]
fn actors_and_enums_are_valid_export_targets_in_the_ast() {
    let p = parse(
        "export pub actor Worker;\nexport pub enum Status;\n         define actor Worker as end\n         define enum Status as Ready end\n         pub fnc main(): int { return 0; }"
    );
    assert_eq!(p.parsed_exports[0].kind, "actor");
    assert_eq!(p.parsed_exports[1].kind, "enum");
}

#[test]
fn bad_targets_and_duplicate_exports_fail_closed() {
    let cases = [
        ("export pub Missing;\npub fnc main(): int { return 0; }", "E_UNBOUND"),
        ("export pub fnc Foo;\n", "E_UNBOUND"),
        ("export pub fnc Foo;\ndefine class Foo as end\npub fnc main(): int { return 0; }", "E_EXPORT_KIND"),
        ("export pub Foo;\nexport pub class Foo;\ndefine class Foo as end\npub fnc main(): int { return 0; }", "E_DUPLICATE"),
        ("export pub[pkg] Ghost;\npub fnc main(): int { return 0; }", "E_UNBOUND"),
    ];
    for (src, want) in cases {
        let err = compile(src).expect_err(src);
        assert_eq!(err.code, want, "{src}: {err}");
    }
}

#[test]
fn invalid_qualifiers_and_missing_separators_do_not_consume_declarations() {
    for src in [
        "export Foo;\npub fnc main(): int { return 0; }",
        "export pub[world] Foo;\npub fnc main(): int { return 0; }",
        "export pub Foo\ndefine class Foo as end\npub fnc main(): int { return 0; }",
        "pub fnc main(): int { export pub Foo; return 0; }",
    ] {
        assert!(compile(src).is_err(), "should reject: {src}");
    }
}

#[test]
fn export_resolves_all_overloads_by_name() {
    let source = "export pub fnc value;\n        fnc value(): int { return 1; }\n        fnc value(int x): int { return x; }\n        pub fnc main(): int { return value() + value(2); }";
    assert!(compile(source).is_ok());
}

#[test]
fn export_name_abi_rejects_non_ascii_and_over_256_bytes() {
    // Forge's accepted boundary and the Wasm admission contract are 256
    // bytes of canonical ASCII, even though the source lexer accepts Unicode.
    let max_name = format!("A{}", "z".repeat(255));
    assert_eq!(max_name.len(), 256);
    let accepted = format!(
        "export pub fnc {max_name};\nfnc {max_name}(): int {{ return 7; }}\npub fnc main(): int {{ return 0; }}"
    );
    assert!(compile(&accepted).is_ok(), "256-byte ASCII export should compile");

    let overlong = format!("A{}", "z".repeat(256));
    assert_eq!(overlong.len(), 257);
    for visibility in ["pub", "pub[pkg]"] {
        let source = format!(
            "export {visibility} fnc {overlong};\nfnc {overlong}(): int {{ return 7; }}\npub fnc main(): int {{ return 0; }}"
        );
        let diagnostic = compile(&source).expect_err("257-byte export must fail");
        assert_eq!(diagnostic.code, "E_EXPORT_NAME_TOO_LONG");
    }

    // Unicode declaration names remain a lexer feature, but may not become
    // language-export names until the shared metadata ABI supports them.
    for name in ["Éclair", "Ωmega"] {
        let source = format!(
            "export pub fnc {name};\nfnc {name}(): int {{ return 7; }}\npub fnc main(): int {{ return 0; }}"
        );
        let diagnostic = compile(&source).expect_err("non-ASCII export must fail");
        assert_eq!(diagnostic.code, "E_EXPORT_NAME");
    }
}

fn read_uleb(bytes: &[u8], offset: &mut usize) -> usize {
    let mut value = 0usize;
    for shift in (0..=63).step_by(7) {
        let byte = *bytes.get(*offset).expect("truncated wasm LEB");
        *offset += 1;
        value |= ((byte & 0x7f) as usize).checked_shl(shift).expect("LEB overflow");
        if byte & 0x80 == 0 { return value; }
    }
    panic!("unterminated wasm LEB");
}

fn read_name(bytes: &[u8], offset: &mut usize) -> String {
    let len = read_uleb(bytes, offset);
    let end = offset.checked_add(len).expect("overflow");
    let value = bytes.get(*offset..end).expect("truncated wasm name");
    *offset = end;
    std::str::from_utf8(value).expect("utf8").to_owned()
}

fn wasm_sections(bytes: &[u8]) -> Vec<(u8, &[u8])> {
    assert!(bytes.starts_with(b"\0asm\x01\0\0\0"), "Wasm header");
    let mut out = vec![];
    let mut pos = 8;
    while pos < bytes.len() {
        let id = bytes[pos];
        pos += 1;
        let len = read_uleb(bytes, &mut pos);
        let end = pos.checked_add(len).expect("section overflow");
        out.push((id, bytes.get(pos..end).expect("section bounds")));
        pos = end;
    }
    out
}

#[test]
fn export_metadata_is_structurally_typed_and_pkg_is_not_a_wasm_export() {
    let wasm = compile(&format!("{EXPORTS}{DECLS}")).expect("compile");
    let sections = wasm_sections(&wasm);
    let mut metadata = vec![];
    let mut host_exports = vec![];
    for (id, section) in sections {
        let mut pos = 0;
        if id == 0 {
            let section_name = read_name(section, &mut pos);
            if section_name == "atto.exports.v1" {
                let count = read_uleb(section, &mut pos);
                for _ in 0..count {
                    let symbol = read_name(section, &mut pos);
                    let kind = *section.get(pos).expect("missing export kind");
                    let visibility = *section.get(pos + 1).expect("missing visibility");
                    pos += 2;
                    metadata.push((symbol, kind, visibility));
                }
                assert_eq!(pos, section.len(), "no trailing metadata bytes");
            }
        } else if id == 7 {
            let count = read_uleb(section, &mut pos);
            for _ in 0..count {
                let symbol = read_name(section, &mut pos);
                pos += 1; // export kind (function/table/memory/global)
                let _index = read_uleb(section, &mut pos);
                host_exports.push(symbol);
            }
            assert_eq!(pos, section.len(), "host export section must be well-formed");
        }
    }
    assert_eq!(metadata, vec![
        ("Bar".to_string(), 1, 2),
        ("Foo".to_string(), 1, 1),
        ("calculate".to_string(), 2, 1),
    ]);
    assert!(host_exports.iter().any(|name| name == "main"));
    for name in ["Bar", "Foo", "calculate"] {
        assert!(!host_exports.iter().any(|s| s == name),
            "language export {name} must not become a host Wasm export");
    }
}

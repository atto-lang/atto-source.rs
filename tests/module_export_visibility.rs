//! Executed browser-compiler regression suite for file-local module exports.
//! Cross-file package/namespace linking is intentionally not claimed here.
use atto_compiler::compile;

fn ok(src: &str) -> Vec<u8> {
    let out = compile(src).unwrap_or_else(|e| panic!("failed to compile: {e}"));
    assert!(out.starts_with(b"\0asm\x01\0\0\0"));
    out
}
fn err(src: &str, expected: &str) {
    let e = compile(src).expect_err("invalid export must never emit a guest");
    assert_eq!(e.code, expected, "{src}: {e}");
    assert!(e.pos.line > 0 && e.pos.col > 0);
}
fn leb(bytes: &[u8], p: &mut usize) -> usize {
    let mut n = 0usize;
    for shift in (0..=63).step_by(7) {
        let byte = *bytes.get(*p).expect("truncated leb");
        *p += 1;
        n |= ((byte & 0x7f) as usize).checked_shl(shift).expect("leb overflow");
        if byte & 0x80 == 0 {
            return n;
        }
    }
    panic!("bad leb");
}
fn wasm_name(bytes: &[u8], p: &mut usize) -> String {
    let len = leb(bytes, p);
    let end = p.checked_add(len).expect("name overflow");
    let name = String::from_utf8(bytes.get(*p..end).expect("name bound").to_vec()).unwrap();
    *p = end;
    name
}
fn sections(wasm: &[u8]) -> Vec<(u8, &[u8])> {
    let mut out = vec![];
    let mut p = 8;
    while p < wasm.len() {
        let id = wasm[p];
        p += 1;
        let len = leb(wasm, &mut p);
        let end = p.checked_add(len).expect("section overflow");
        out.push((id, wasm.get(p..end).expect("section bound")));
        p = end;
    }
    out
}
fn module_markers(wasm: &[u8]) -> Vec<(String, u8)> {
    let mut result = vec![];
    for (id, bytes) in sections(wasm) {
        if id != 0 {
            continue;
        }
        let mut p = 0;
        if wasm_name(bytes, &mut p) != "atto.exports.modules.v1" {
            continue;
        }
        for _ in 0..leb(bytes, &mut p) {
            let name = wasm_name(bytes, &mut p);
            let visibility = *bytes.get(p).expect("missing visibility");
            p += 1;
            result.push((name, visibility));
        }
        assert_eq!(p, bytes.len(), "module section must end exactly");
    }
    result
}
fn host_exports(wasm: &[u8]) -> Vec<String> {
    let mut names = vec![];
    for (id, bytes) in sections(wasm) {
        if id != 7 {
            continue;
        }
        let mut p = 0;
        for _ in 0..leb(bytes, &mut p) {
            names.push(wasm_name(bytes, &mut p));
            p += 1; // host export kind
            let _ = leb(bytes, &mut p); // index
        }
        assert_eq!(p, bytes.len());
    }
    names
}
const BODY: &str = "define module Math as\n pub fnc double(int x): int { return x * 2; }\n fnc secret(): int { return 7; }\nend\npub fnc main(): int { return Math.double(21); }\n";

#[test]
fn inferred_public_class_exports_equal_explicit_class_exports() {
    let declarations = "define class Receipt as end\npub fnc main(): int { return 0; }\n";
    let inferred = ok(&format!("export pub Receipt;\n{declarations}"));
    let explicit = ok(&format!("export pub class Receipt;\n{declarations}"));
    assert_eq!(inferred, explicit, "kind inference must not alter class visibility or Wasm");
}

#[test]
fn module_exports_are_order_independent_and_not_host_wasm_exports() {
    let forward = ok(&format!("export pub module Math;\n{BODY}"));
    let trailing = ok(&format!("{BODY}export pub module Math;\n"));
    assert_eq!(forward, trailing);
    assert_eq!(module_markers(&forward), vec![("Math".to_string(), 1)]);
    let host = host_exports(&forward);
    assert!(host.iter().any(|s| s == "main"));
    assert!(!host.iter().any(|s| s == "Math" || s == "double" || s == "secret"));
}
#[test]
fn inferred_package_export_and_inline_public_module() {
    let pkg = ok(&format!("export pub[pkg] Math;\n{BODY}"));
    assert_eq!(module_markers(&pkg), vec![("Math".into(), 2)]);
    let inline = ok(&BODY.replace("define module", "define pub module"));
    assert_eq!(module_markers(&inline), vec![("Math".into(), 1)]);
    let explicit = ok(&format!(
        "export pub module Math;\n{}",
        BODY.replace("define module", "define pub module")
    ));
    assert_eq!(inline, explicit, "matching inline and declarative pub cannot duplicate metadata");
}
#[test]
fn exports_cannot_cross_kind_or_publish_private_members() {
    err(&format!("export pub fnc Math;\n{BODY}"), "E_EXPORT_KIND");
    err("export pub module Absent;\npub fnc main(): int { return 0; }", "E_UNBOUND");
    err(&format!("export pub fnc secret;\n{BODY}"), "E_UNBOUND");
    err(&format!("export pub module Math;\nexport pub Math;\n{BODY}"), "E_DUPLICATE");
    err(
        &format!(
            "export pub[pkg] module Math;\n{}",
            BODY.replace("define module", "define pub module")
        ),
        "E_VISIBILITY",
    );
    err(
        &format!("export pub module Math;\n{}", BODY.replace("Math.double(21)", "Math.secret()")),
        "E_PRIVATE",
    );
}
#[test]
fn nested_exports_cannot_escape_block_or_nominal_scope() {
    err(
        "fnc helper(): int { return 42; }\npub fnc main(): int { export pub fnc helper; return 0; }",
        "E_SYNTAX",
    );
    err(
        "define module M as\n export pub fnc helper;\n pub fnc helper(): int { return 42; }\nend\npub fnc main(): int { return 0; }",
        "E_SYNTAX",
    );
    err(
        "define class C as\n pub get(): int { return 2; }\n export pub class C;\nend\npub fnc main(): int { return 0; }",
        "E_SYNTAX",
    );
    err(
        "fnc helper(): int { return 42; }\nfnc main() -> int\n export pub fnc helper;\n return 0;\nend",
        "E_SYNTAX",
    );
    err(
        "define class C as\n pub get() -> int\n   return 2;\n end\n export pub class C;\nend\npub fnc main(): int { return 0; }",
        "E_SYNTAX",
    );
}
#[test]
fn export_function_overload_sets_remain_arity_based() {
    ok("export pub fnc choose;\nfnc choose(): int { return 1; }\nfnc choose(int x): int { return x; }\npub fnc main(): int { return choose() + choose(41); }");
}
#[test]
fn effect_annotated_brace_body_does_not_spoil_trailing_export_scope() {
    let src = "fnc answer() -> int throws(Error) { return 42; }\nexport pub fnc answer;\n";
    let tokens = atto_compiler::lexer::lex(src).expect("tokenization");
    let lowered = atto_compiler::visibility_lower::lower(tokens).expect("export should remain file level");
    assert_eq!(lowered.exports.len(), 1);
    assert_eq!(lowered.exports[0].name, "answer");
}

#[test]
fn namespace_headers_and_cross_file_imports_fail_closed_until_linked() {
    assert!(compile("use namespace shop;\nexport pub module Math;\ndefine module Math as\n pub fnc answer(): int { return 42; }\nend\npub fnc main(): int { return 0; }").is_err());
    assert!(compile("import module Math from \"./math.atto\";\nexport pub module Math;\npub fnc main(): int { return 0; }").is_err());
}

#[test]
fn export_identifier_abi_accepts_256_ascii_bytes_and_rejects_out_of_range() {
    let name = format!("A{}", "z".repeat(255));
    let source = format!("export pub fnc {name};\nfnc {name}(): int {{ return 1; }}\npub fnc main(): int {{ return 0; }}\n");
    ok(&source);
    let long_name = format!("A{}", "z".repeat(256));
    let long_source = format!("export pub fnc {long_name};\nfnc {long_name}(): int {{ return 1; }}\npub fnc main(): int {{ return 0; }}\n");
    err(&long_source, "E_EXPORT_NAME_TOO_LONG");
    let unicode = "export pub fnc Éclair;\nfnc Éclair(): int { return 1; }\npub fnc main(): int { return 0; }";
    err(unicode, "E_EXPORT_NAME");
}

#[test]
fn v1_wasm_manifest_is_structurally_decodable_and_host_exports_unchanged() {
    let wasm = ok("export pub fnc answer;\nfnc answer(): int { return 42; }\npub fnc main(): int { return answer(); }");
    let mut found = vec![];
    for (id, bytes) in sections(&wasm) {
        if id != 0 {
            continue;
        }
        let mut p = 0;
        if wasm_name(bytes, &mut p) != "atto.exports.v1" {
            continue;
        }
        for _ in 0..leb(bytes, &mut p) {
            let symbol = wasm_name(bytes, &mut p);
            let kind = *bytes.get(p).expect("kind");
            p += 1;
            let visibility = *bytes.get(p).expect("visibility");
            p += 1;
            found.push((symbol, kind, visibility));
        }
        assert_eq!(p, bytes.len(), "no trailing v1 manifest bytes");
    }
    assert_eq!(found, vec![("answer".into(), 2, 1)]);
    assert!(!host_exports(&wasm).contains(&"answer".to_string()));
    assert!(host_exports(&wasm).contains(&"main".to_string()));
}

#[test]
fn actor_and_enum_exports_bind_to_distinct_declaration_kinds() {
    let source = "export pub actor Worker;\nexport pub[pkg] enum Status;\ndefine actor Worker as end\ndefine enum Status as Ready end\npub fnc main(): int { return 0; }";
    let mut lowered = atto_compiler::visibility_lower::lower(atto_compiler::lexer::lex(source).expect("lex")).expect("export syntax");
    let module = atto_compiler::module_lower::lower_with_manifest(lowered.tokens).expect("module pass");
    let parsed = atto_compiler::parser::Parser::new(module.tokens)
        .parse_program()
        .expect("declarations");
    atto_compiler::visibility_lower::resolve_exports(&mut lowered.exports, &parsed, &module.modules)
        .expect("bind all locally owned actor/enum exports");
    assert!(lowered.exports.iter().any(|e| e.name == "Worker" && e.kind == "actor"));
    assert!(lowered.exports.iter().any(|e| e.name == "Status" && e.kind == "enum"));
}

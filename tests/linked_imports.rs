//! Executed Rust integration tests for the bounded in-memory linker.
//! Linking is explicit and single-hop. Package identity is supplied by a
//! trusted host resolver, never inferred from untrusted Atto source text.
use atto_compiler::{linker::{compile_linked, LinkUnit}, lexer, parser::Parser};

const EXPORTED: &str = r#"
use namespace accounting;
export pub module Math;
define module Math as
    pub fnc double(int x): int { return x * 2; }
    fnc hidden(): int { return 11; }
end
"#;
const PKG_EXPORTED: &str = r#"
use namespace accounting;
export pub[pkg] module Math;
define module Math as
    pub fnc double(int x): int { return x * 2; }
    fnc hidden(): int { return 11; }
end
"#;
const INLINE_PUBLIC: &str = r#"
use namespace accounting;
define pub module Math as
    pub fnc double(int x): int { return x * 2; }
end
"#;
const PRIVATE: &str = r#"
use namespace accounting;
define module Math as
    pub fnc double(int x): int { return x * 2; }
end
"#;
const MAIN: &str = r#"
use namespace storefront;
import module Math from "./math.atto";
pub fnc main(): int { return Math.double(21); }
"#;

fn unit<'a>(path: &'a str, package: &'a str, source: &'a str) -> LinkUnit<'a> {
    LinkUnit { path, package, source }
}
fn run(main: &str, dep: &str, package: &str) -> Result<Vec<u8>, atto_compiler::diag::Diagnostic> {
    compile_linked("src/main.atto", &[
        unit("src/main.atto", "owner.storefront", main),
        unit("src/math.atto", package, dep),
    ])
}
fn ok(main: &str, dep: &str, package: &str) -> Vec<u8> {
    let out = run(main, dep, package).unwrap_or_else(|e| panic!("expected compiled guest: {e}"));
    assert!(out.starts_with(b"\0asm\x01\0\0\0"));
    out
}
fn err(main: &str, dep: &str, package: &str, code: &str) {
    let e = run(main, dep, package).expect_err("invalid import must never emit guest");
    assert_eq!(e.code, code, "{e}");
    assert!(e.pos.line > 0 && e.pos.col > 0);
}

#[test]
fn cross_namespace_public_import_builds_guest_and_is_deterministic() {
    let a = ok(MAIN, EXPORTED, "another.library");
    let b = ok(MAIN, EXPORTED, "another.library");
    assert_eq!(a, b);
    let reversed = compile_linked("src/main.atto", &[
        unit("src/math.atto", "another.library", EXPORTED),
        unit("src/main.atto", "owner.storefront", MAIN),
    ]).expect("input list order must not affect linked guest");
    assert_eq!(a, reversed);
}

#[test]
fn package_private_import_requires_matching_canonical_package() {
    err(MAIN, PKG_EXPORTED, "another.library", "E_VISIBILITY");
    assert!(run(MAIN, PKG_EXPORTED, "owner.storefront").is_ok());
}

#[test]
fn inline_public_module_is_linkable_but_plain_private_module_is_not() {
    ok(MAIN, INLINE_PUBLIC, "another.library");
    err(MAIN, PRIVATE, "owner.storefront", "E_VISIBILITY");
}

#[test]
fn module_member_visibility_is_checked_after_linking() {
    let source = MAIN.replace("Math.double(21)", "Math.hidden()");
    err(&source, EXPORTED, "another.library", "E_PRIVATE");
}

#[test]
fn missing_imports_and_wrong_owner_names_fail() {
    let miss = compile_linked("src/main.atto", &[unit("src/main.atto","owner.storefront",MAIN)])
        .expect_err("missing file cannot create guest");
    assert_eq!(miss.code, "E_UNBOUND");
    let source = EXPORTED.replace("module Math", "module Counter");
    err(MAIN, &source, "another.library", "E_UNBOUND");
}

#[test]
fn importing_does_not_authorize_reexport_of_an_unowned_module() {
    let source = MAIN.replace("pub fnc main", "export pub module Math;\npub fnc main");
    err(&source, EXPORTED, "another.library", "E_VISIBILITY");
    let source2 = MAIN.replace("pub fnc main", "export pub[pkg] Math;\npub fnc main");
    err(&source2, EXPORTED, "another.library", "E_VISIBILITY");
}

#[test]
fn canonical_file_paths_forbid_traversal_and_synthetic_aliasing() {
    let attempted = MAIN.replace("./math.atto","../math.atto");
    err(&attempted, EXPORTED, "another.library", "E_IMPORT_PATH");
    let attempted = MAIN.replace("./math.atto","././math.atto");
    err(&attempted, EXPORTED, "another.library", "E_IMPORT_PATH");
    let duplicate = compile_linked("src/main.atto", &[
        unit("src/main.atto","owner.storefront",MAIN),
        unit("src/main.atto","owner.storefront",MAIN),
    ]).expect_err("duplicate file identity");
    assert_eq!(duplicate.code, "E_DUPLICATE");
    let invalid = compile_linked("src/../main.atto", &[
        unit("src/../main.atto","owner.storefront",MAIN),
    ]).expect_err("path traversal");
    assert_eq!(invalid.code, "E_IMPORT_PATH");
}

#[test]
fn imports_are_first_position_declarations_not_nested_expressions() {
    let late = r#"pub fnc main(): int { return 0; }
import module Math from "./math.atto";"#;
    err(late, EXPORTED, "another.library", "E_SYNTAX");
    let nested = r#"pub fnc main(): int {
      import module Math from "./math.atto";
      return 0;
    }"#;
    err(nested, EXPORTED, "another.library", "E_SYNTAX");
    let repeated = format!("import module Math from \"./math.atto\";\n{MAIN}");
    err(&repeated, EXPORTED, "another.library", "E_SYNTAX");
}

#[test]
fn cyclic_and_transitive_imports_fail_closed() {
    let cycle = MAIN.replace("./math.atto", "./main.atto");
    err(&cycle, EXPORTED, "owner.storefront", "E_IMPORT_CYCLE");
    let two_hops = format!("import module Math from \"./math.atto\";\n{}", EXPORTED);
    err(MAIN, &two_hops, "another.library", "E_SYNTAX");
    let dep = r#"use namespace accounting;
import module Helpers from "./helpers.atto";
export pub module Math;
define module Math as
    pub fnc double(int x): int { return x * 2; }
end"#;
    err(MAIN, dep, "another.library", "E_UNSUPPORTED");
}

#[test]
fn source_cannot_smuggle_foreign_or_nonmodule_code_across_link_boundary() {
    let injected = format!("{}\nfnc helper(): int {{ return 42; }}", EXPORTED);
    err(MAIN, &injected, "another.library", "E_UNSUPPORTED");
    let bad = format!("{}\nexternal fnc host_clock_ns(): int;",EXPORTED);
    err(MAIN, &bad, "another.library", "E_UNSUPPORTED");
}

#[test]
fn namespace_headers_and_comments_are_not_symbol_imports() {
    let with_comment = format!("// import module Fake from \"./fake.atto\";\n{}", MAIN);
    ok(&with_comment, EXPORTED, "another.library");
    let bad_header = MAIN.replace("use namespace storefront;", "use namespace storefront;\nuse namespace stolen;");
    err(&bad_header, EXPORTED, "another.library", "E_SYNTAX");
}

#[test]
fn direct_ast_parser_and_pre_lowering_consume_the_same_export_grammar() {
    let source = "export pub Foo;\nexport pub[pkg] fnc calculate;\ndefine class Foo as end\nfnc calculate(): int { return 42; }\npub fnc main(): int { return calculate(); }";
    let tokens = lexer::lex(source).expect("lexer");
    let ast = Parser::new(tokens.clone()).parse_program().expect("direct parser");
    assert_eq!(ast.parsed_exports.len(), 2);
    assert_eq!(ast.parsed_exports[0].name, "Foo");
    assert_eq!(ast.parsed_exports[1].kind, "fnc");
    assert_eq!(ast.parsed_exports[1].visibility, atto_compiler::visibility_lower::Visibility::Package);
    let lexical = atto_compiler::visibility_lower::lower(tokens).expect("lowering");
    assert_eq!(lexical.exports.len(), 2);
    for (p, l) in ast.parsed_exports.iter().zip(lexical.exports.iter()) {
        assert_eq!(p.name, l.name);
        assert_eq!(p.kind, l.kind);
        assert_eq!(p.visibility, l.visibility);
        assert_eq!(p.pos, l.pos);
    }
    // A raw parsed export cannot bypass the linker/checker. Binding it
    // explicitly produces the same validated AST export table.
    let unbound = atto_compiler::check::check(&ast).expect_err("unchecked AST export");
    assert_eq!(unbound.code, "E_UNBOUND");
    let mut resolved = ast.clone();
    atto_compiler::visibility_lower::bind_program_exports(&mut resolved, vec![], &[])
        .expect("bind direct AST using same resolver as compile()");
    assert!(resolved.parsed_exports.is_empty());
    assert_eq!(resolved.exports.len(), 2);
    assert_eq!(resolved.exports[0].name, "Foo");
    assert_eq!(resolved.exports[1].name, "calculate");
    let guest = atto_compiler::compile(source).expect("same source through checked compiler");
    assert!(guest.starts_with(b"\0asm\x01\0\0\0"));
}

#[test]
fn direct_parser_rejects_duplicate_and_invalid_scope_exports() {
    let repeated = "export pub Foo;\nexport pub Foo;\ndefine class Foo as end";
    let error = Parser::new(lexer::lex(repeated).unwrap()).parse_program().unwrap_err();
    assert_eq!(error.code, "E_DUPLICATE");
    let nested = "define class Foo as\nexport pub Foo;\nend";
    assert!(Parser::new(lexer::lex(nested).unwrap()).parse_program().is_err());
    let invalid = "export pub[guest] Foo;\ndefine class Foo as end";
    let error = Parser::new(lexer::lex(invalid).unwrap()).parse_program().unwrap_err();
    assert_eq!(error.code, "E_SYNTAX");
}

//! Real Rust scalar frontend admission, not singleton guest implementation.
use oreslang_source_rs::frontend::parse;

#[test]
fn singleton_declarations_have_stable_fail_closed_diagnostic() {
    for declaration in [
        "define singleton S as end",
        "define pub singleton S as end",
        "pub define singleton S as end",
        "define /* inert */ singleton S as end",
        "define pub /* inert */ singleton S as end",
        "define module M as define singleton S as end end",
        "pub fnc main(): int { return 7; } define singleton S as end",
        "define singleton S as state { let n: int = 0 } end",
    ] {
        let err = parse(declaration).expect_err(declaration);
        assert_eq!(err.class, "singleton_unsupported", "{declaration}: {err}");
        assert!(err.message.contains("ORES-SINGLETON-UNSUPPORTED"));
        assert_eq!(err.offset, declaration.find("singleton").unwrap(), "{declaration}");
    }
}
#[test]
fn singleton_is_contextual_and_comment_text_is_inert() {
    for source in [
        "fnc singleton(): int { return 7; }",
        "fnc main(int singleton): int { return singleton; }",
        "pub fnc main(): int { return singleton(); } fnc singleton(): int { return 7; }",
        "// define singleton Fake as end\nfnc main(): int { return 7; }",
        "/* define pub singleton Fake as end */ fnc main(): int { return 7; }",
        "fnc main(): int { return 7; } // define singleton Fake",
    ] {
        parse(source).unwrap_or_else(|e| panic!("ordinary source denied {source}: {e}"));
    }
}
#[test]
fn unsupported_singleton_cannot_enter_native_code_generation() {
    let src = "pub fnc main(): int { return 7; }\ndefine singleton S as end";
    for result in [
        oreslang_source_rs::compile(src),
        oreslang_source_rs::compile_checked_i64(src),
        oreslang_source_rs::compile_with_effect_ir(src).map(|(bytes, _)| bytes),
    ] {
        assert!(result.unwrap_err().contains("ORES-SINGLETON-UNSUPPORTED"));
    }
}

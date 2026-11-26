//! Portable Unicode identifier and Boolean NOT compiler admission.
use atto_compiler::{compile, lexer::{lex, Tok}};

#[test]
fn lex_unicode_scalars_and_preserve_negation_boundary() {
    let tokens = lex("Φ φ π ε ρ 価格 價格 変数 𠮷 x𠮷2 〇 ¬ ! !=").unwrap();
    for (i, name) in ["Φ","φ","π","ε","ρ","価格","價格","変数","𠮷","x𠮷2","〇"].iter().enumerate() {
        assert_eq!(tokens[i].tok, Tok::Ident(name.to_string()));
    }
    assert_eq!(tokens[11].tok, Tok::UnicodeNot);
    assert_eq!(tokens[12].tok, Tok::Bang);
    assert_eq!(tokens[13].tok, Tok::NotEq);
}

#[test]
fn unicode_guest_compiles_and_not_lowers_like_bang() {
    let unicode = "pub fnc main(): int { val Φ = 10; val φ = 20; val 数量 = 30; val 𠮷 = 40; if ¬false && ¬¬true then return Φ + φ + 数量 + 𠮷; else return 0; fi }";
    let ascii_not = unicode.replace('¬', "!");
    let bytes = compile(unicode).expect("Unicode guest must compile");
    assert!(bytes.starts_with(b"\0asm\x01\0\0\0"));
    assert_eq!(bytes, compile(&ascii_not).unwrap(), "NOT spelling must not affect emitted guest");
}

#[test]
fn rejects_invalid_negation_and_hidden_bidi() {
    for source in [
        "pub fnc main(): bool { return ¬1; }",
        "pub fnc main(): int { return 未定義; }",
        "pub fnc main(bool b): bool { return b¬; }",
        "pub fnc main(bool b): bool { return b ¬eq true; }",
        "pub fnc main(bool b): bool { return b ¬= true; }",
    ] {
        assert!(compile(source).is_err(), "{source}");
    }
    for source in ["// \u{202e}", "/* \u{2066} */", "\"\u{200f}\""] {
        assert_eq!(lex(source).unwrap_err().code, "E_SOURCE_BIDI");
    }
}

//! Integer bitwise operator admission and grammar collision regression.
use atto_compiler::{compile, io_shorthand, lexer};

fn source(expr: &str) -> String {
    format!("pub fnc main(): int {{ return {expr}; }}")
}

#[test]
fn bitwise_operations_compile_and_emit_wasm() {
    for expr in [
        "12 & 10", "12 | 10", "12 ^^ 10", "~12",
        "8 <<< 2", "32 >>> 2", "-8 ashr 1",
        "1 + 2 <<< 3", "64 >>> 1 + 1",
        "1 <<< 64", "-1 >>> 1",
        "12 & 10 | 2 ^^ 6", "(1 | 2) <<< 1",
    ] {
        let bytes = compile(&source(expr)).unwrap_or_else(|e| panic!("{expr}: {e}"));
        assert_eq!(&bytes[..8], b"\0asm\x01\0\0\0", "{expr}");
    }
}

#[test]
fn compound_assignment_uses_local_mutability_and_original_value() {
    for assignment in [
        "x &= 10", "x |= 2", "x ^^= 7",
        "x <<<= 2", "x >>>= 1", "x ashr= 1",
    ] {
        let src = format!("pub fnc main(): int {{ let x: int = 12; {assignment}; return x; }}");
        compile(&src).unwrap_or_else(|e| panic!("{assignment}: {e}"));
        let readonly = format!("pub fnc main(): int {{ val x: int = 12; {assignment}; return x; }}");
        assert_eq!(compile(&readonly).unwrap_err().code, "E_IMMUTABLE", "{assignment}");
    }
}

#[test]
fn bitwise_is_strictly_integer_only() {
    for expr in ["1.0 & 2", "1 | true", "true ^^ false", "1.5 <<< 2",
                 "1 >>> 1.0", "1 ashr false", "~true"] {
        assert_eq!(compile(&source(expr)).unwrap_err().code, "E_TYPE", "{expr}");
    }
}

#[test]
fn triple_shift_tokens_cannot_be_mistaken_for_stream_io() {
    for input in ["x <<< 3;", "x >>> 3;", "x <<<= 2;", "x >>>= 2;"] {
        let toks = lexer::lex(input).unwrap();
        assert!(io_shorthand::inspect(&toks).unwrap().is_empty(), "{input}");
    }
    assert_eq!(io_shorthand::inspect(&lexer::lex("stdio.out << 1;").unwrap()).unwrap().len(), 1);
    assert_eq!(io_shorthand::inspect(&lexer::lex("source >> val x: A<B<C<D>>>;").unwrap()).unwrap().len(), 1);
}

#[test]
fn binary_literals_match_signed_int_range_and_reject_malformed_prefixes() {
    for literal in ["0b101100", "0b001100", "0B1011", "0b1_010"] {
        compile(&source(literal)).unwrap_or_else(|e| panic!("{literal}: {e}"));
    }
    // This exact source is also compiled inside the wasm32 compiler by Node.
    compile(include_str!("fixtures/pass/192-bitwise-playground.atto")).unwrap();
    for literal in ["0b", "0B", "0b_", "0b2", "0b10foo", "0x10"] {
        assert_eq!(lexer::lex(literal).unwrap_err().code, "E_INTEGER_LITERAL", "{literal}");
    }
    let overflow = format!("0b1{}", "0".repeat(64));
    assert_eq!(lexer::lex(&overflow).unwrap_err().code, "E_INTEGER_RANGE");
}

#[test]
fn existing_io_and_boolean_operators_are_unchanged() {
    compile("pub fnc main(): int { stdio.out << 42; return 0; }").unwrap();
    compile("pub fnc main(): int { if true | false then return 1; fi return 0; }").unwrap();
    // Normative v1 compatibility: legacy unparenthesized condition '|' is
    // boolean OR; parenthesized value '|' is integer bitwise OR.
    compile("pub fnc main(): int { let bits: int = (1 | 2); if bits eq 3 then return 1; fi return 0; }").unwrap();
    compile("pub fnc main(): int { if (1 | 2) eq 3 then return 1; fi return 0; }").unwrap();
    compile("pub fnc main(): int { if (true || false) then return 1; fi return 0; }").unwrap();
    compile("pub fnc main(): int { if true && false || true then return 1; fi return 0; }").unwrap();
    for bad in ["1 ^ 2", "1 ^ ^ 2", "1 ^^^ 2"] {
        assert!(compile(&source(bad)).is_err(), "{bad}");
    }
    assert!(compile(&source("1 << 2")).is_err()); // Not a bit shift
    assert!(compile(&source("8 >> 2")).is_err()); // Not a bit shift
}

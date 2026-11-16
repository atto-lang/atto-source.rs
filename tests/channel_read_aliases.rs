//! `readch` and `c >> val` are real checked Channel<int> reads,
//! not dynamic-read calls, scalar sentinel reads or parser-only features.
use atto_compiler::{compile,ast::{ExprKind,Stmt},lexer,parser::Parser};

fn source(body:&str)->String {
    format!("pub susp fnc read_channel_guest_v1(Channel<int> c): Result<Option<int>,IoError> {{ {body} }} pub fnc main():int {{ return 0; }}")
}
fn marker(bytes:&[u8],text:&str)->bool {
    bytes.windows(text.len()).any(|w| w==text.as_bytes())
}

#[test]
fn explicit_and_binary_aliases_lower_to_exactly_one_prefix_read() {
    for body in [
        "val result = readch c; return result;",
        "val result = (readch c); return result;",
        "return readch c;",
        "c >> val result; return result;",
        "c >> let result; return result;",
        "c >> val result: Result<Option<int>,IoError>; return result;",
        "c >> val Result<Option<int>,IoError> result; return result;",
    ] {
        let src=source(body);
        let program=Parser::new(lexer::lex(&src).unwrap()).parse_program().expect(body);
        let expr=match &program.funcs[0].body[0] {
            Stmt::Bind {init,..} => init,
            Stmt::Return(Some(value),_) => value,
            _ => panic!("missing read expression in {body}"),
        };
        assert!(matches!(&expr.kind,ExprKind::PrefixRead(_)),
            "one checked read per source expression: {body}");
        let wasm=compile(&src).unwrap_or_else(|e| panic!("{body}: {e}"));
        assert!(wasm.starts_with(b"\0asm\x01\0\0\0"));
        for check in ["atto.channel_int","channel_int_nb_read_v1",
             "channel_int_await_read_v1","cooperate"] {
            assert!(marker(&wasm,check),"{body} did not import {check}");
        }
    }
}

#[test]
fn incorrect_effect_receiver_and_result_reject_without_guest() {
    for (src,code) in [
        ("pub fnc read_channel_guest_v1(Channel<int> c): Result<Option<int>,IoError> { return readch c; } pub fnc main():int {return 0;}", "E_SUSPENDS_REQUIRED"),
        ("pub susp fnc read_channel_guest_v1(int c): Result<Option<int>,IoError> { return readch c; } pub fnc main():int {return 0;}", "E_IO_READABLE"),
        ("pub susp fnc read_channel_guest_v1(Channel<int> c): int { return readch c; } pub fnc main():int {return 0;}", "E_TYPE"),
        ("pub fnc read_channel_guest_v1(Channel<int> c): Result<Option<int>,IoError> { c >> val item; return item; } pub fnc main():int {return 0;}", "E_SUSPENDS_REQUIRED"),
        ("pub susp fnc read_channel_guest_v1(Channel<int> c): int { c >> val item; return item; } pub fnc main():int {return 0;}", "E_TYPE"),
        ("pub susp fnc read_channel_guest_v1(Channel<int> c): Result<Option<int>,IoError> { c >> const item; return item; } pub fnc main():int {return 0;}", "E_CONST_EFFECT"),
    ] {
        let d=compile(src).expect_err("invalid read must have no Wasm artifact");
        assert_eq!(d.code,code,"source: {src} -> {d}");
    }
}

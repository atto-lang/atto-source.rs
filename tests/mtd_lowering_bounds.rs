//! Adversarial source-span and complexity checks for the *single* merged
//! class mtd lowerer. The token stream and checked AST must never be bypassed.
use atto_compiler::{compile, lexer::lex, parser::Parser};
use atto_compiler::target_syntax::{lower_class_receivers, parse_target_declarations};

fn many_classes(n: usize) -> String {
    let mut src = String::new();
    for i in 0..n {
        src.push_str(&format!("define class C{i} as final int value: {i}; end\n"));
        src.push_str(&format!(
            "pub mtd (self: C{i}) answer() -> int :: self.value end\n"
        ));
    }
    src
}

#[test]
fn class_token_lowering_preserves_many_owners_without_quadratic_scans() {
    // Enough owners to stress the old O(classes * tokens + receivers * tokens)
    // loops, while staying inside the documented 64 KiB source cap.
    let source = many_classes(96);
    let tokens = lex(&source).expect("valid token limit");
    let headers = parse_target_declarations(&tokens).expect("valid classes");
    assert_eq!(headers.class_definitions.len(), 96);
    assert_eq!(headers.receivers.len(), 96);
    let lowered = lower_class_receivers(&tokens, &headers).expect("one-pass lowering");
    assert!(lowered.len() <= tokens.len(), "lowering may not inflate tokens");
    let program = Parser::new(lowered).parse_program().expect("canonical production AST");
    assert_eq!(program.classes.len(), 96);
    for (i, class) in program.classes.iter().enumerate() {
        assert_eq!(class.name, format!("C{i}"));
        assert_eq!(class.methods.len(), 1);
        assert_eq!(class.methods[0].name, "answer");
    }
}

#[test]
fn malformed_and_overlapping_receiver_span_ledgers_fail_closed() {
    let source = many_classes(2);
    let tokens = lex(&source).unwrap();
    let good = parse_target_declarations(&tokens).unwrap();
    assert!(lower_class_receivers(&tokens, &good).is_ok());

    let mut overlap = good.clone();
    overlap.receiver_sites[0].end = overlap.receiver_sites[1].end;
    assert_eq!(lower_class_receivers(&tokens, &overlap).unwrap_err().code, "E_SYNTAX");

    let mut duplicate_class_end = good.clone();
    duplicate_class_end.class_ends[0] = duplicate_class_end.class_ends[1];
    assert_eq!(lower_class_receivers(&tokens, &duplicate_class_end).unwrap_err().code, "E_SYNTAX");

    let mut class_end_in_omission = good.clone();
    class_end_in_omission.class_ends[0] = good.receiver_sites[0].declaration_start;
    assert_eq!(lower_class_receivers(&tokens, &class_end_in_omission).unwrap_err().code, "E_SYNTAX");

    let mut out_of_bounds = good.clone();
    out_of_bounds.receiver_sites[0].end = tokens.len() + 10;
    assert_eq!(lower_class_receivers(&tokens, &out_of_bounds).unwrap_err().code, "E_SYNTAX");

    let mut missing = good.clone();
    missing.receiver_sites.pop();
    assert_eq!(lower_class_receivers(&tokens, &missing).unwrap_err().code, "E_SYNTAX");
}

#[test]
fn lexer_tokens_ignore_comment_and_string_pretending_to_access_self() {
    let src = r#"
        define class C as final int value: 7; end
        // self.value = 444; and val alias = self; are not source statements
        pub mtd (self: C) answer() -> int {
            val label: string = "self.value = 444; val alias = self;";
            return self.value;
        }
        pub fnc main(): int { val c = new C(); return c.answer(); }
    "#;
    let wasm = compile(src).expect("comments and string literals cannot forge receiver tokens");
    assert_eq!(&wasm[..8], b"\\0asm\\x01\\0\\0\\0");
}

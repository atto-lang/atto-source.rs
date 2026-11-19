//! New nominal declaration/constructor syntax must use the existing checked
//! class constructor path, never an unlocked state or identity-bearing struct.
use atto_compiler::{compile, lexer::lex};
use atto_compiler::target_syntax::{parse_target_declarations, lower_class_receivers};

const CLASS: &str = r#"
    declare class Counter;
    define class Counter as
        pub let int value = 0;
    end
    pub constructor (Counter)(int initial): self {
        self.value = initial;
    }
    pub mtd (Counter self) get_value(): int {
        return self.value;
    }
    pub fnc main(): int {
        val counter = new Counter(42);
        return counter.get_value();
    }
"#;

#[test]
fn forward_class_constructor_and_receiver_lower_through_existing_checked_ast() {
    let toks = lex(CLASS).unwrap();
    let h = parse_target_declarations(&toks).unwrap();
    assert_eq!(h.class_forwards, ["Counter"]);
    assert_eq!(h.classes, ["Counter"]);
    assert_eq!(h.constructors.len(), 1);
    assert_eq!(h.receivers.len(), 1);
    let lowered = lower_class_receivers(&toks, &h).unwrap();
    assert!(!lowered.iter().any(|t| matches!(&t.tok, atto_compiler::lexer::Tok::Ident(s) if s == "declare")));
    let wasm = compile(CLASS).expect("existing class ctor/receiver must emit real checked Wasm");
    assert_eq!(&wasm[0..8], &[0, 97, 115, 109, 1, 0, 0, 0]);
    assert_eq!(wasm, compile(CLASS).unwrap(), "deterministic guest output");
}

#[test]
fn constructor_header_uses_nominal_owner_and_self_result() {
    let source = "define class C as end pub constructor (C)(): self {} pub fnc main(): int { val c = new C(); return 7; }";
    let h = parse_target_declarations(&lex(source).unwrap()).unwrap();
    assert_eq!(h.constructors.len(), 1);
    assert_eq!(h.constructors[0].owner, "C");
    assert!(compile(source).is_ok(), "stateless class ctor must use existing AST");
}

#[test]
fn guarded_class_state_parses_but_never_downgrades_to_unlocked_fields() {
    let state = r#"
        declare class Counter;
        define state on class Counter as { let int count = 0; }
        pub constructor (Counter)(int initial): self {
          self.handle_state(critical-write |s| -> {s.count = initial;});
        }
    "#;
    let h = parse_target_declarations(&lex(state).unwrap()).unwrap();
    assert_eq!(h.state_owners, ["Counter"]);
    assert_eq!(compile(state).unwrap_err().code, "E_UNSUPPORTED");
    let d = compile("declare class C; define state on class C as {").unwrap_err();
    assert_eq!(d.code, "E_EOF");
    let d = compile("define state on class Missing as {}").unwrap_err();
    assert_eq!(d.code, "E_UNBOUND");
}

#[test]
fn struct_receivers_cannot_silently_use_heap_class_shape_lowering() {
    for source in [
        "define struct Point as int x; end mtd (Point self) get(): int {return self.x;}",
        "define struct Point as int x; end constructor (Point)(int x): self {self.x = x;}",
        "declare struct Point; pub mtd (Point self) get(): int {return 1;}",
    ] {
        let d = compile(source).unwrap_err();
        assert_eq!(d.code, "E_UNSUPPORTED", "{source}: {d}");
    }
}

#[test]
fn constructor_ownership_linking_and_duplicate_headers_fail_closed() {
    for (source, expected) in [
        ("pub constructor (Missing)(): self {}", "E_UNBOUND"),
        ("declare actor A; pub constructor (A)(): self {}", "E_ACTOR"),
        ("define class C as constructor() {} end constructor (C)(): self {}", "E_DUPLICATE"),
        ("define class C as end constructor (C)(): self {} constructor (C)(int x): self {}", "E_DUPLICATE"),
        ("declare class C; constructor (C)(): self {}", "E_UNSUPPORTED"),
    ] {
        let d = compile(source).unwrap_err();
        assert_eq!(d.code, expected, "{source}: {d}");
    }
}

#[test]
fn constructor_whitespace_aliases_have_same_tokens_ast_and_guest_bytes() {
    let headers = [
        "constructor (Counter)(int initial)",
        "constructor (Counter) (int initial)",
        "constructor(Counter) (int initial)",
    ];
    let mut canonical_tokens = None;
    let mut canonical_lowered = None;
    let mut canonical_wasm = None;
    for header in headers {
        let source = format!(r#"
            define class Counter as
                pub let int value = 0;
            end
            pub {header}: self {{
                self.value = initial;
            }}
            pub fnc main(): int {{
                val counter = new Counter(42);
                return counter.value;
            }}
        "#);
        let tokens = lex(&source).unwrap();
        let parsed = parse_target_declarations(&tokens).unwrap();
        assert_eq!(parsed.classes, ["Counter"]);
        assert_eq!(parsed.constructors.len(), 1);
        assert_eq!(parsed.constructors[0].owner, "Counter");
        assert!(parsed.constructors[0].public);
        let lexemes = tokens.iter().map(|t| t.tok.clone()).collect::<Vec<_>>();
        if let Some(ref baseline) = canonical_tokens {
            assert_eq!(&lexemes, baseline, "constructor spacing changed token sequence");
        } else {
            canonical_tokens = Some(lexemes);
        }
        let lowered = lower_class_receivers(&tokens, &parsed).unwrap();
        let lexemes = lowered.iter().map(|t| t.tok.clone()).collect::<Vec<_>>();
        if let Some(ref baseline) = canonical_lowered {
            assert_eq!(&lexemes, baseline, "constructor spacing changed lowered AST tokens");
        } else {
            canonical_lowered = Some(lexemes);
        }
        let wasm = compile(&source).expect("all three forms must compile to checked class guest");
        assert_eq!(&wasm[..8], &[0, 97, 115, 109, 1, 0, 0, 0]);
        if let Some(ref baseline) = canonical_wasm {
            assert_eq!(&wasm, baseline, "constructor spacing changed Wasm guest bytes");
        } else {
            canonical_wasm = Some(wasm);
        }
    }
}

#[test]
fn whitespace_variations_do_not_create_constructor_overloads() {
    let source = r#"
        define class Counter as end
        constructor (Counter)(): self {}
        constructor(Counter) (): self {}
    "#;
    let error = parse_target_declarations(&lex(source).unwrap()).unwrap_err();
    assert_eq!(error.code, "E_DUPLICATE");
    let error = compile(source).unwrap_err();
    assert_eq!(error.code, "E_DUPLICATE");

    for source in [
        "define class Counter as end constructor (Counter)(int initial): self",
        "define class Counter as end constructor (Counter) (int initial): self",
        "define class Counter as end constructor(Counter) (int initial): self",
    ] {
        assert!(compile(source).is_err(), "header without a body is incomplete");
    }
}

#[test]
fn existing_standalone_state_syntax_bypasses_out_of_line_class_state_proposal() {
    let valid = include_str!("fixtures/pass/180-standalone-state-critical.atto");
    let guest = compile(valid).expect("standalone state must retain its old checked lowering");
    assert_eq!(&guest[..8], b"\0asm\x01\0\0\0");
    let invalid = include_str!("fixtures/fail/180-state-unguarded-self.atto");
    let diagnostic = compile(invalid).expect_err("unguarded state must still fail closed");
    assert_eq!(diagnostic.code, "E_STATE_ACCESS", "{diagnostic}");
}

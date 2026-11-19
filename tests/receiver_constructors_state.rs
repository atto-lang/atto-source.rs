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
    assert!(wasm.starts_with(b"\\0asm\\x01\\0\\0\\0"));
    assert_eq!(wasm, compile(CLASS).unwrap(), "deterministic guest output");
}

#[test]
fn constructor_header_uses_nominal_owner_and_self_result() {
    let source = "define class C as end constructor (C)(): self {}";
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

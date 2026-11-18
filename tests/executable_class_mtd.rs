//! Executable immutable class mtd uses the same checker and ABI as inline
//! methods, while mutable class and actor receivers stay fail-closed.
use atto_compiler::{compile, lexer::lex, parser::Parser};
use atto_compiler::target_syntax::{parse_target_declarations,lower_class_receivers};

const SOURCE: &str = r#"
    define class Calculator as
      final int seed: 3;
      pub fnc six() -> int :: 5 end
    end
    pub mtd (self: Calculator) one(): int { return self.seed + 2; }
    pub mtd (Calculator self) two(): int -> { return self.seed + 2; }
    pub mtd (Calculator) three() -> int { return self.seed + 2; }
    pub mtd (Calculator _) four() -> int
      const x: int = self.seed;
      ^ x + 2;
    end
    pub mtd (self: Calculator) five() -> int :: let int x = self.seed, x + 2 end
    pub fnc main(): int {
      val c = new Calculator();
      return c.one() + c.two() + c.three() + c.four() + c.five() + c.six();
    }
"#;

#[test]
fn all_class_receiver_aliases_lower_to_real_checked_methods() {
    let tokens = lex(SOURCE).unwrap();
    let headers = parse_target_declarations(&tokens).unwrap();
    assert_eq!(headers.class_definitions.len(), 1);
    assert_eq!(headers.receivers.len(), 5);
    assert_eq!(headers.receiver_sites.len(), 5);
    let lowered = lower_class_receivers(&tokens, &headers).unwrap();
    let program = Parser::new(lowered).parse_program().unwrap();
    assert_eq!(program.classes[0].methods.len(), 6);
    assert_eq!(program.funcs.len(), 1);
    let guest = compile(SOURCE).unwrap();
    assert_eq!(&guest[..8], &[0, 97, 115, 109, 1, 0, 0, 0]);
    assert_eq!(guest,compile(SOURCE).unwrap(),"deterministic checked guest");
}

#[test]
fn mutable_class_receivers_require_verified_mutable_self_abi() {
    let src = r#"
        define class C as
          let int count = 0;
        end
        mtd (mut self: C) increment() -> int :: 1 end
        pub fnc main(): int { return 0; }
    "#;
    let diagnostic = compile(src).unwrap_err();
    assert_eq!(diagnostic.code, "E_UNSUPPORTED", "{diagnostic}");
}

#[test]
fn namespace_and_actor_mtd_never_fall_back_to_unsafe_class_dispatch() {
    let namespace = r#"
      use namespace N;
      define class C as end
      mtd (self: C) get() -> int :: 5 end
      pub fnc main(): int { return 0; }
    "#;
    let d = compile(namespace).unwrap_err();
    assert_eq!(d.code,"E_UNSUPPORTED","{d}");
    let actor = r#"
      declare actor Actor;
      define class C as end
      mtd (self: C) get() -> int :: 5 end
      pub fnc main(): int { return 0; }
    "#;
    let d = compile(actor).unwrap_err();
    assert_eq!(d.code,"E_UNSUPPORTED","{d}");
}

#[test]
fn malformed_class_mtd_bodies_fail_under_real_parser() {
    for (body, code) in [
      ("-> int :: 3, 5", "E_EOF"),
      ("-> int :: 3, end", "E_SYNTAX"),
      ("-> int :: 3 5 end", "E_SYNTAX"),
      (": int -> 5 end", "E_SYNTAX"),
    ] {
      let source = format!("define class C as end mtd (self: C) bad() {body} pub fnc main(): int {{ return 0; }}");
      let d = compile(&source).unwrap_err();
      assert_eq!(d.code,code,"{source}: {d}");
    }
}

#[test]
fn non_public_receiver_stays_private_after_lowering() {
    let source = r#"
      define class C as end
      mtd (self: C) secret() -> int :: 5 end
      pub fnc main(): int {
        val c = new C();
        return c.secret();
      }
    "#;
    let d = compile(source).unwrap_err();
    assert_eq!(d.code,"E_PRIVATE","{d}");
}

#[test]
fn class_mtd_duplicate_signature_is_a_semantic_error() {
    let source = r#"
      define class C as
        get() -> int :: 5 end
      end
      mtd (C _) get() -> int :: 6 end
      pub fnc main(): int { return 0; }
    "#;
    let d = compile(source).unwrap_err();
    assert_eq!(d.code,"E_DUPLICATE","{d}");
}

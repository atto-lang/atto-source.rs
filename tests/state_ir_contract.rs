use atto_compiler::{ast::Type, check::{self,TExpr,TStmt}, lexer, parser::Parser};
use std::fs;

fn typed() -> check::TProgram {
    let source = fs::read_to_string("tests/fixtures/pass/180-standalone-state-critical.atto").unwrap();
    let parsed = Parser::new(lexer::lex(&source).unwrap()).parse_program().unwrap();
    check::check(&parsed).unwrap()
}

#[test]
fn critical_write_retains_guard_and_protected_opcodes_in_typed_ir() {
    let program = typed();
    let increment = program.funcs.iter().find(|f| f.name == "Counter.increment").unwrap();
    let mut found = false;
    for statement in &increment.body {
        if let TStmt::StateCriticalWrite(body) = statement {
            assert_eq!(body.len(), 1);
            let TStmt::StateFieldWrite { obj, ty, value, .. } = &body[0] else {
                panic!("state assignment was erased into ordinary FieldSet");
            };
            assert!(matches!(obj.ty, Type::Class(_)));
            assert_eq!(*ty, Type::Int);
            assert!(matches!(&value.e, TExpr::Bin(..)));
            found = true;
        }
    }
    assert!(found, "protected write must remain a distinct typed scope");
}

#[test]
fn critical_read_is_not_lowered_to_ordinary_field_get() {
    let program = typed();
    let read = program.funcs.iter().find(|f| f.name == "Counter.read_count").unwrap();
    let result = read.body.iter().find_map(|stmt| match stmt {
        TStmt::Return(Some(value)) => Some(value), _ => None,
    }).expect("read_count must return a state projection");
    assert!(matches!(result.e, TExpr::StateFieldRead { .. }),
        "critical-read must preserve typed ownership authority through IR");
}

#[test]
fn state_constructor_can_still_initialize_storage_without_a_guest_lease() {
    let program = typed();
    let constructor = program.funcs.iter().find(|f| f.name == "Counter.new").unwrap();
    assert!(constructor.body.iter().any(|stmt| matches!(stmt,TStmt::FieldSet {..})));
    assert!(!constructor.body.iter().any(|stmt| matches!(stmt,TStmt::StateFieldWrite {..})),
        "creation must not forge a state write loan");
}

#[test]
fn critical_read_block_never_escalates_to_exclusive_state_write() {
    let source = r#"
define state Counter as
  state { let int count: 0; }
  pub bad(): void {
    handle_state(critical-read |s| -> {});
  }
end
pub fnc main(): int { return 0; }
"#;
    let diagnostic = atto_compiler::compile(source).unwrap_err();
    assert_eq!(diagnostic.code, "E_STATE_BORROW", "{diagnostic}");
}


#[test]
fn returned_write_snapshots_scalar_under_one_lease_and_returns_outside_it() {
    let source = fs::read_to_string("tests/fixtures/pass/state-write-return-scalar.atto").unwrap();
    let parsed = Parser::new(lexer::lex(&source).unwrap()).parse_program().unwrap();
    let program = check::check(&parsed).unwrap();
    for (name, expected) in [("Counter.increment", Type::Int), ("Counter.mark", Type::Bool)] {
        let method = program.funcs.iter().find(|f| f.name == name).unwrap();
        let block = method.body.iter().find_map(|stmt| match stmt {
            TStmt::Block(body) => Some(body),
            _ => None,
        }).expect("returned critical region must remain a block");
        assert_eq!(block.len(), 2);
        let TStmt::StateCriticalWrite(body) = &block[0] else {
            panic!("missing exclusive lease");
        };
        assert_eq!(body.len(), 2);
        assert!(matches!(&body[0], TStmt::StateFieldWrite { .. }));
        let TStmt::Set(slot, value) = &body[1] else {
            panic!("callback result must be stored before release");
        };
        assert_eq!(value.ty, expected);
        assert!(matches!(&value.e, TExpr::StateFieldRead { .. }));
        let TStmt::Return(Some(result)) = &block[1] else {
            panic!("method return must follow the critical scope");
        };
        assert_eq!(result.ty, expected);
        assert!(matches!(&result.e, TExpr::Local(s) if s == slot));
    }
}


#[test]
fn state_aliases_preserve_identical_wasm_for_every_admitted_mode() {
    let template = r#"
define state container Account as
  state { let int balance: 0; let int transactions: 0; }
  pub deposit(int amount): void {
    WRITE |view| -> {
      view.balance = view.balance + amount;
      view.transactions = view.transactions + 1;
    });
  }
  pub add(int amount): int {
    return WRITE |view| -> {
      view.balance = view.balance + amount;
      return view.balance;
    });
  }
  pub read_balance(): int {
    return READ |view| -> view.balance);
  }
  pub read_transactions(): int {
    return READ |view| -> { return view.transactions; });
  }
end
pub fnc main(): int {
  val a = new Account();
  a.deposit(40);
  val balance = a.add(2);
  return balance + a.read_balance() + a.read_transactions();
}
"#;
    // The existing compiler API is also used by the fixture runner. Compare
    // binary output rather than just checking whether each spelling parses.
    let source = template.replace("WRITE ", "handle_state(critical-write ")
                         .replace("READ ", "handle_state(critical-read ");
    let baseline = atto_compiler::compile(&source).unwrap();
    for (write, read) in [
        ("state_w(", "state_ro("),
        ("handle_state_w(", "handle_state_ro("),
        ("self.state_w(", "self.state_ro("),
        ("self.handle_state_w(", "self.handle_state_ro("),
        ("self.handle_state(critical-write ", "self.handle_state(critical-read "),
        ("handle_state(critical ", "handle_state(critical-read "),
    ] {
        let source = template.replace("WRITE ", write).replace("READ ", read);
        let candidate = atto_compiler::compile(&source).unwrap();
        assert!(baseline == candidate, "alias changed compiled output: {write} / {read}");
    }
}

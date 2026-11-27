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

#[test]
fn independently_mutated_ast_cannot_bypass_critical_shape_admission() {
    use atto_compiler::ast::{Expr, ExprKind, Stmt};
    let source = fs::read_to_string("tests/fixtures/pass/180-standalone-state-critical.atto").unwrap();
    let parsed = Parser::new(lexer::lex(&source).unwrap()).parse_program().unwrap();
    for variant in 0..4 {
        let mut corrupt = parsed.clone();
        let method = corrupt.classes.iter_mut().find(|c| c.name=="Counter").unwrap()
            .methods.iter_mut().find(|m| m.name=="increment").unwrap();
        let (body,pos) = method.body.iter_mut().find_map(|s| match s {
            Stmt::StateWrite {body,pos} => Some((body,*pos)), _=>None
        }).unwrap();
        let scalar = Expr {kind:ExprKind::Int(Some(1)),pos};
        match variant {
            0=>body.insert(0,Stmt::Return(None,pos)),
            1=>body.insert(0,Stmt::Expr(Expr {kind:ExprKind::Call("stdio.println".into(),vec![scalar]),pos})),
            2=>body.insert(0,Stmt::StateWrite{body:vec![],pos}),
            _=>{
                let Stmt::FieldAssign{value,..}=&mut body[0] else {panic!("expected field assignment")};
                *value=Expr{kind:ExprKind::StateWriteValue(vec![],Box::new(scalar)),pos};
            }
        }
        match check::check(&corrupt) {
            Err(diagnostic)=>assert_eq!(diagnostic.code,"E_STATE_BORROW","variant {variant}: {diagnostic}"),
            Ok(_)=>panic!("forged AST variant {variant} bypassed callback shape validation"),
        }
    }
}

#[test]
fn corrupted_typed_state_region_cannot_emit_guest_bytes() {
    use atto_compiler::{check::TE, wasm};
    for variant in 0..5 {
        let mut program=typed();
        let method=program.funcs.iter_mut().find(|f| f.name=="Counter.increment").unwrap();
        let body=method.body.iter_mut().find_map(|s| match s {
            TStmt::StateCriticalWrite(body)=>Some(body), _=>None
        }).unwrap();
        let scalar=TE{e:TExpr::I64(1),ty:Type::Int};
        match variant {
            0=>body.insert(0,TStmt::Return(None)),
            1=>body.insert(0,TStmt::Expr(TE{e:TExpr::Call(0,vec![]),ty:Type::Int})),
            2=>body.insert(0,TStmt::StateCriticalWrite(vec![])),
            3=>{let TStmt::StateFieldWrite{obj,..}=&mut body[0] else{panic!("field")};obj.e=TExpr::Local(1);},
            _=>{let TStmt::StateFieldWrite{value,..}=&mut body[0] else{panic!("field")};*value=TE{e:TExpr::StateWriteValue{body:vec![],result:Box::new(scalar)},ty:Type::Int};}
        }
        assert!(std::panic::catch_unwind(|| wasm::emit(&program)).is_err(),"forged typed region {variant} emitted");
    }
}

#[test]
fn protected_state_emission_authenticates_nominal_receiver_and_field_layout() {
    use atto_compiler::{check::TE, wasm};
    assert!(!wasm::emit(&typed()).is_empty());
    for variant in 0..5 {
        let mut program = typed();
        let method = program.funcs.iter_mut().find(|f| f.name == "Counter.increment").unwrap();
        let body = method.body.iter_mut().find_map(|s| match s {
            TStmt::StateCriticalWrite(body) => Some(body), _ => None,
        }).unwrap();
        let TStmt::StateFieldWrite { obj, offset, ty, value } = &mut body[0] else {
            panic!("expected protected assignment");
        };
        match variant {
            0 => *offset = 8, // aligned but outside this one-field layout
            1 => obj.ty = Type::Class(u32::MAX),
            2 => {
                *ty = Type::Bool;
                *value = TE { e: TExpr::Bool(true), ty: Type::Bool };
            }
            3 => method.params[0] = Type::Class(u32::MAX),
            _ => method.params[0] = Type::Int,
        }
        assert!(std::panic::catch_unwind(|| wasm::emit(&program)).is_err(),
            "corrupt protected write layout {variant} emitted bytes");
    }
    for variant in 0..3 {
        let mut program = typed();
        let method = program.funcs.iter_mut().find(|f| f.name == "Counter.read_count").unwrap();
        let read = method.body.iter_mut().find_map(|s| match s {
            TStmt::Return(Some(value)) => Some(value), _ => None,
        }).unwrap();
        let TExpr::StateFieldRead { obj, offset } = &mut read.e else { panic!("expected read"); };
        match variant {
            0 => *offset = 8,
            1 => obj.ty = Type::Class(u32::MAX),
            _ => read.ty = Type::Bool,
        }
        assert!(std::panic::catch_unwind(|| wasm::emit(&program)).is_err(),
            "corrupt protected read layout {variant} emitted bytes");
    }
}

#[test]
fn protected_snapshot_authenticates_slots_and_generated_tuple_constructor() {
    use atto_compiler::wasm;
    let source = fs::read_to_string("tests/fixtures/pass/state-owned-tuple-snapshot.atto").unwrap();
    let parsed = Parser::new(lexer::lex(&source).unwrap()).parse_program().unwrap();
    let original = check::check(&parsed).unwrap();
    assert!(!wasm::emit(&original).is_empty());
    for variant in 0..5 {
        let mut program = original.clone();
        let method = program.funcs.iter_mut().find(|f| f.name == "Account.snapshot").unwrap();
        let value = method.body.iter_mut().find_map(|s| match s {
            TStmt::Return(Some(value)) => Some(value), _ => None,
        }).unwrap();
        let TExpr::StateSnapshot { ctor, fields } = &mut value.e else { panic!("expected snapshot"); };
        match variant {
            0 => fields[0].1 = 16, // aligned but not an Account field
            1 => fields[0].0 = Type::Bool,
            2 => *ctor = u32::MAX,
            3 => *ctor = program.main,
            _ => value.ty = Type::Class(u32::MAX),
        }
        assert!(std::panic::catch_unwind(|| wasm::emit(&program)).is_err(),
            "corrupt protected snapshot {variant} emitted bytes");
    }
}

#[test]
fn protected_region_rejects_forged_scalar_local_and_ticket_slot() {
    use atto_compiler::{check::TE, wasm};
    assert!(!wasm::emit(&typed()).is_empty());
    for variant in 0..4 {
        let mut program = typed();
        let method = program.funcs.iter_mut().find(|f| f.name == "Counter.increment").unwrap();
        let ticket = (method.params.len() + method.locals.len()) as u32;
        if variant == 3 { method.params[1] = Type::Bool; }
        let body = method.body.iter_mut().find_map(|s| match s {
            TStmt::StateCriticalWrite(body) => Some(body), _ => None,
        }).unwrap();
        let TStmt::StateFieldWrite { value, .. } = &mut body[0] else { panic!("expected write"); };
        let slot = match variant { 0 => ticket, 1 => u32::MAX, 2 => 0, _ => 1 };
        *value = TE { e: TExpr::Local(slot), ty: Type::Int };
        assert!(std::panic::catch_unwind(|| wasm::emit(&program)).is_err(),
            "forged scalar local {variant} emitted bytes");
    }
}

#[test]
fn protected_result_cannot_overwrite_parameter_or_host_ticket() {
    use atto_compiler::wasm;
    let source = fs::read_to_string("tests/fixtures/pass/state-write-return-scalar.atto").unwrap();
    let parsed = Parser::new(lexer::lex(&source).unwrap()).parse_program().unwrap();
    let original = check::check(&parsed).unwrap();
    assert!(!wasm::emit(&original).is_empty());
    for variant in 0..3 {
        let mut program = original.clone();
        let method = program.funcs.iter_mut().find(|f| f.name == "Counter.increment").unwrap();
        let ticket = (method.params.len() + method.locals.len()) as u32;
        let block = method.body.iter_mut().find_map(|s| match s {
            TStmt::Block(b) => Some(b), _ => None,
        }).unwrap();
        let TStmt::StateCriticalWrite(body) = &mut block[0] else { panic!("expected write region"); };
        let Some(TStmt::Set(slot, _)) = body.last_mut() else { panic!("expected detached result"); };
        *slot = match variant { 0 => 1, 1 => ticket, _ => u32::MAX };
        assert!(std::panic::catch_unwind(|| wasm::emit(&program)).is_err(),
            "forged result store {variant} emitted bytes");
    }
}

#[test]
fn forged_protected_schema_cannot_gain_ordinary_class_or_reference_authority() {
    use atto_compiler::ast::{FieldBinding, FieldMode, FieldVisibility};
    let source = fs::read_to_string("tests/fixtures/pass/180-standalone-state-critical.atto").unwrap();
    let original = Parser::new(lexer::lex(&source).unwrap()).parse_program().unwrap();
    assert!(check::check(&original).is_ok());
    for variant in 0..13 {
        let mut parsed = original.clone();
        let c = parsed.classes.iter_mut().find(|c| c.name == "Counter").unwrap();
        match variant {
            0 => c.fields[0].visibility = FieldVisibility::Public,
            1 => c.fields[0].visibility = FieldVisibility::Package,
            2 => c.fields[0].mode = FieldMode::Final,
            3 => c.fields[0].binding = FieldBinding::Let,
            4 => c.fields[0].referent_mutable = true,
            5 => c.fields[0].init = None,
            6 => c.fields[0].ty = Type::Str,
            7 => c.fields[0].ty = Type::Float,
            8 => c.fields.clear(),
            9 => c.methods[0].is_static = true,
            10 => c.methods[0].name = "state_w".into(),
            11 => c.methods[0].params[0].name = "state_ro".into(),
            _ => c.methods[0].params[0].name = "s".into(),
        }
        match check::check(&parsed) {
            Err(d) => assert_eq!(d.code, "E_STATE_BORROW", "schema {variant}: {d}"),
            Ok(_) => panic!("forged schema {variant} admitted"),
        }
    }
}

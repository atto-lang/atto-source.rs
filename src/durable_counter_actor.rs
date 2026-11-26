//! Narrow source-level durable Counter actor lowering for the existing
//! checked scalar broker ABI. This is not a general-purpose actor backend.
//! Parse the complete actor, require the exact typed increment handler,
//! then lower the exclusive state transition to a pure scalar guest.
//! The broker owns activation, state, admission, receipts and publication.
use crate::ast::{ActorPersistenceMode, BinOp, Expr, ExprKind, FieldMode,
    FieldVisibility, Stmt, Type};
use crate::diag::{Diagnostic, Pos};

fn denied(pos: Pos) -> Diagnostic {
    Diagnostic::new(pos, "E_DURABLE_COUNTER_ACTOR_ABI",
        "requires one private zero-initialized int Counter state field and a single ActorMail<int> receive increment; other actors require verified general durable lowering")
}
fn self_field(expr: &Expr, field: &str) -> bool {
    matches!(&expr.kind, ExprKind::Member(base, name)
        if name == field && matches!(&base.kind, ExprKind::SelfRef))
}
fn payload(expr: &Expr, mail: &str) -> bool {
    matches!(&expr.kind, ExprKind::Member(base, name)
        if name == "value" && matches!(&base.kind, ExprKind::Var(n) if n == mail))
}

// Versioned, compiler-emitted schema metadata. A broker must authenticate
// the complete module and compare the exact descriptor before using it.
// This section is not itself an authorization capability.
fn append_uleb(out: &mut Vec<u8>, mut value: usize) {
    loop {
        let byte = (value & 0x7f) as u8;
        value >>= 7;
        out.push(if value == 0 { byte } else { byte | 0x80 });
        if value == 0 { break; }
    }
}

fn append_schema(module: &mut Vec<u8>, schema: &crate::actor_persistence::ActorStateSchema,
                 initial: &[u8]) -> Result<(), Diagnostic> {
    const SECTION: &[u8] = b"atto.durable.state.v1";
    let mut data = Vec::new();
    append_uleb(&mut data, SECTION.len());
    data.extend_from_slice(SECTION);
    data.push(1); // descriptor format version
    append_uleb(&mut data, schema.actor_name.len());
    data.extend_from_slice(schema.actor_name.as_bytes());
    append_uleb(&mut data, schema.fields[0].name.len());
    data.extend_from_slice(schema.fields[0].name.as_bytes());
    data.push(1); // int scalar tag (admitted single-field profile)
    append_uleb(&mut data, initial.len());
    data.extend_from_slice(initial);
    let mut encoded = vec![0]; // Wasm custom section
    append_uleb(&mut encoded, data.len());
    encoded.extend_from_slice(&data);
    if module.len().saturating_add(encoded.len()) > crate::wasm::MAX_OUTPUT {
        return Err(Diagnostic::new(Pos::default(), "E_DURABLE_COUNTER_ACTOR_ABI",
            "durable schema metadata exceeds module output budget"));
    }
    module.extend_from_slice(&encoded);
    Ok(())
}

pub fn compile(source: &str) -> Result<Vec<u8>, Diagnostic> {
    if source.len() > crate::SOURCE_CAP {
        return Err(Diagnostic::new(Pos::default(), "E_SOURCE_LIMIT", "durable actor source exceeds cap"));
    }
    let tokens = crate::lexer::lex(source)?;
    let mut program = crate::parser::Parser::new(tokens).parse_program()?;
    if program.actors.len() != 1 || !program.funcs.is_empty() ||
       !program.classes.is_empty() || !program.enums.is_empty() ||
       program.actor_interface.is_some() || !program.type_names.is_empty() ||
       !program.array_types.is_empty() {
        return Err(denied(Pos::default()));
    }
    let actor = &program.actors[0];
    if actor.name != "Counter" || actor.persistence != ActorPersistenceMode::Durable ||
       !actor.persistence_hooks.is_empty() || !actor.methods.is_empty() ||
       actor.fields.len() != 1 {
        return Err(denied(actor.pos));
    }
    let field = &actor.fields[0];
    if field.ty != Type::Int || field.mode != FieldMode::ActorState ||
       field.visibility != FieldVisibility::Private || field.referent_mutable ||
       !matches!(field.init.as_ref().map(|e| &e.kind), Some(ExprKind::Int(Some(0)))) {
        return Err(denied(field.pos));
    }
    // Reuse the existing compile-time persistable schema gate.
    let schema = crate::actor_persistence::ActorStateSchema::for_actor(actor)?;
    let initial = schema.encode(&[crate::actor_persistence::StateScalar::Int(0)])?;
    let receive = actor.receive.as_ref().ok_or_else(|| denied(actor.pos))?;
    if receive.payload != Type::Int {
        return Err(denied(receive.pos));
    }
    let [Stmt::FieldAssign { target, field: assigned, value, .. }] = receive.body.as_slice() else {
        return Err(denied(receive.pos));
    };
    if !matches!(&target.kind, ExprKind::SelfRef) || assigned != &field.name {
        return Err(denied(receive.pos));
    }
    let ExprKind::Binary(BinOp::Add, lhs, rhs) = &value.kind else {
        return Err(denied(value.pos));
    };
    if !self_field(lhs, &field.name) || !payload(rhs, &receive.mail) {
        return Err(denied(value.pos));
    }
    // Check the original, fully parsed handler with the normal actor checker.
    // The volatile clone is used ONLY for semantic checking, never emitted:
    // the ordinary durable actor gate remains fail-closed.
    program.actors[0].persistence = ActorPersistenceMode::Volatile;
    let main_tokens = crate::lexer::lex("pub fnc main()->int { return 0; }")?;
    let main = crate::parser::Parser::new(main_tokens).parse_program()?;
    program.funcs = main.funcs;
    crate::check::check(&program)?;

    // Exact admitted AST is self.field = self.field + mail.value.
    // The broker's fixed counter domain is 0..1e9 and its op=2 is a
    // broker read, not a second user-defined receive or a guest effect.
    // No arbitrary user code is interpolated into generated source.
    let mut module = crate::compile_durable_scalar(r#"
fnc durable_transition(int state, int op, int delta) -> int {
    if (state < 0 || state > 1000000000) { return -1; } fi
    if (op eq 2) {
        if (delta neq 0) { return -1; } fi
        return state;
    } fi
    if (op neq 1 || delta <= 0 || delta > 1000000000) { return -1; } fi
    if (state > 1000000000 - delta) { return -1; } fi
    return state + delta;
}
pub fnc main()->int { return 0; }
"#)?;
    append_schema(&mut module, &schema, &initial)?;
    Ok(module)
}

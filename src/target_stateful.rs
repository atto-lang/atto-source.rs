//! Opt-in *real source-body* lowering for one sealed critical-write callback.
//!
//! No general mtd codegen is authorized. Only the fully typed, exact AST form
//! `s.<int_field> = s.<int_field> + <int_parameter>` is admitted. The initial
//! value is the field's actual compiled initializer, not a synthetic value.
//! The resulting Wasm has three audited instruction-level release paths.
//! Production compile() and receiver-instance preview remain fail closed.
use crate::ast::{ExprKind, FieldMode, Type, UnOp};
use crate::diag::{Diagnostic,Pos};
use crate::lexer::{Tok,Token};
use crate::target_cleanup::StateLeaseMode;
use crate::target_cleanup_cfg::lower_verified_cleanup_cfg;
use crate::target_typed::TargetType;

pub(crate) struct StatefulShape {
    pub owner: String,
    pub method: String,
    pub field: String,
    pub initial: i64,
}
fn unsupported(pos:Pos,reason:&'static str)->Diagnostic{
    Diagnostic::new(pos,"E_UNSUPPORTED",reason)
}
fn literal_int(tokens:&crate::ast::Expr)->Option<i64>{
    match &tokens.kind {
        ExprKind::Int(Some(v)) if *v<=i64::MAX as u64=>Some(*v as i64),
        ExprKind::Unary(UnOp::Neg,inner)=>match &inner.kind {
            ExprKind::Int(Some(v)) if *v<=i64::MAX as u64+1=>
                Some((*v as i128).wrapping_neg() as i64),
            _=>None,
        },
        _=>None,
    }
}
fn exact_callback(body:&[Token], binder:&str, field:&str, param:&str)->bool {
    let expected=[
        Tok::LBrace,
        Tok::Ident("self".into()),Tok::Dot,Tok::Ident("handle_state".into()),
        Tok::LParen,Tok::Ident("critical".into()),Tok::Minus,
        Tok::Ident("write".into()),Tok::Pipe,Tok::Ident(binder.into()),
        Tok::Pipe,Tok::Arrow,Tok::LBrace,
        Tok::Ident(binder.into()),Tok::Dot,Tok::Ident(field.into()),
        Tok::Assign,Tok::Ident(binder.into()),Tok::Dot,
        Tok::Ident(field.into()),Tok::Plus,Tok::Ident(param.into()),
        Tok::Semi,Tok::RBrace,Tok::RParen,Tok::Semi,Tok::RBrace
    ];
    body.len()==expected.len()&&body.iter().zip(expected.iter())
        .all(|(token,want)|token.tok==*want)
}

/// Restricted real stateful callback compiler. The Wasm instruction paths are
/// checked against the existing cleanup CFG, and the source shape is audited
/// *after* full syntax / typed checking. There is no fallback or inference of
/// missing information from incomplete source facts.
pub fn compile_stateful_method_preview(source:&str)->Result<Vec<u8>,Diagnostic>{
    if source.len()>crate::SOURCE_CAP {
        return Err(Diagnostic::new(Pos{line:1,col:1},"E_SOURCE_LIMIT",
            "stateful preview source exceeds cap"));
    }
    let tokens=crate::lexer::lex(source)?;
    if tokens.iter().any(|t|matches!(&t.tok,Tok::Ident(name)
        if name=="fnc"||name=="import")) {
        return Err(unsupported(Pos::default(),
            "stateful preview cannot link other functions or modules"));
    }
    let headers=crate::target_syntax::parse_target_declarations(&tokens)?;
    let typed=crate::target_typed::lower_target_signatures(&headers)?;
    let actor_ast=crate::target_typed::build_actor_program(&headers,&typed);
    crate::check::check_target_actor_interface(&actor_ast)?;
    if headers.actors.len()!=1 || headers.definitions.len()!=1 ||
       headers.receivers.len()!=1 || headers.receiver_bodies.len()!=1 ||
       typed.receivers.len()!=1 || typed.receiver_cleanup.len()!=1 ||
       !headers.imports.is_empty() || headers.state_assignments.len()!=1 {
        return Err(unsupported(Pos::default(),
            "stateful preview requires one concrete actor and one exact state method"));
    }
    let owner=&headers.definitions[0];
    let header=&headers.receivers[0];
    let method=&typed.receivers[0];
    let plan=&typed.receiver_cleanup[0];
    if header.owner!=owner.name || method.actor!=0 || !header.mutable ||
       !method.mutable || method.arity!=1 || method.params.len()!=1 ||
       method.params[0].ty!=TargetType::Int ||
       method.returns!=TargetType::Void ||
       owner.receive.is_some() || !owner.methods.is_empty() ||
       owner.fields.len()!=1 || plan.leases.len()!=1 ||
       plan.leases[0].mode!=StateLeaseMode::ExclusiveWrite ||
       !plan.leases[0].checked_arithmetic || !typed.receivers[0].effects.writes_state ||
       typed.receivers[0].effects.unproved_call ||
       typed.receivers[0].effects.may_suspend {
        return Err(unsupported(header.pos,
            "stateful preview only supports one typed int-field critical-write increment"));
    }
    let field=&owner.fields[0];
    if field.ty!=Type::Int || field.mode!=FieldMode::ActorState ||
       !headers.state_assignments.iter().all(|a|
           a.receiver==0 && a.field==field.name && !a.compound) {
        return Err(unsupported(field.pos,"stateful preview requires one mutable int field"));
    }
    let initial=literal_int(&field.init).ok_or_else(||
        unsupported(field.pos,"stateful preview requires a constant i64 field initializer"))?;
    let binder=&headers.state_assignments[0].binder;
    if !exact_callback(&headers.receiver_bodies[0].tokens,binder,
        &field.name,&method.params[0].name) {
        return Err(unsupported(header.pos,
            "stateful callback must be exactly: s.field = s.field + integer_parameter"));
    }
    if [&owner.name,&header.name,&field.name].iter().any(|n|
        n.len()>128 || n.is_empty() ||
        !n.bytes().enumerate().all(|(i,c)|c==b'_' ||
            (c>=b'A'&&c<=b'Z') || (c>=b'a'&&c<=b'z') ||
            (i>0 && c>=b'0'&&c<=b'9'))) {
        return Err(unsupported(header.pos,"invalid or overlong stateful ABI identifier"));
    }
    let verified=lower_verified_cleanup_cfg(plan)?;
    let shape=StatefulShape{owner:owner.name.clone(),
        method:header.name.clone(),field:field.name.clone(),initial};
    let bytes=crate::wasm::emit_stateful_method_preview(&verified,&shape)?;
    if bytes.len()>crate::wasm::MAX_OUTPUT {
        return Err(unsupported(header.pos,"stateful preview output exceeds 1 MiB"));
    }
    Ok(bytes)
}

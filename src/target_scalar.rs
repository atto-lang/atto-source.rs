//! Deliberately opt-in *stateless* scalar receiver preview.
//!
//! A preview export has only the explicit i64 parameters, not an actor
//! instance or actor scheduler. This is real executable Wasm for a small
//! validated method subset, NOT actor dispatch, state isolation or AOT mtd.
//! Production compile() stays fail-closed for every out-of-line mtd.
use crate::diag::{Diagnostic, Pos};
use crate::lexer::{Tok, Token};
use crate::target_syntax::ReceiverBodyTokens;
use crate::target_typed::{TargetType, TypedReceiver};

#[derive(Clone, Debug)]
pub enum ScalarExpr {
    Param(u32),
    Const(i64),
    Neg(Box<ScalarExpr>),
    Add(Box<ScalarExpr>, Box<ScalarExpr>),
    Sub(Box<ScalarExpr>, Box<ScalarExpr>),
    Mul(Box<ScalarExpr>, Box<ScalarExpr>),
}

#[derive(Clone, Debug)]
pub struct ScalarMethod {
    pub owner: String,
    pub name: String,
    pub arity: usize,
    pub pos: Pos,
    pub expr: ScalarExpr,
}

fn error(pos: Pos, reason: impl Into<String>) -> Diagnostic {
    Diagnostic::new(pos, "E_UNSUPPORTED", reason)
}

struct ExprParser<'a> {
    toks: &'a [Token],
    at: usize,
    receiver: &'a TypedReceiver,
    nodes: usize,
}

impl ExprParser<'_> {
    fn pos(&self) -> Pos {
        self.toks.get(self.at).or_else(|| self.toks.last())
            .map_or(Pos::default(), |t| t.pos)
    }

    fn node(&mut self, expr: ScalarExpr) -> Result<ScalarExpr, Diagnostic> {
        self.nodes += 1;
        if self.nodes > 128 {
            return Err(error(self.pos(),"scalar preview expression node limit"));
        }
        Ok(expr)
    }

    fn expr(&mut self, precedence: u8, depth: usize) -> Result<ScalarExpr, Diagnostic> {
        if depth > 32 {
            return Err(Diagnostic::new(self.pos(),"E_NESTING",
                "scalar preview expression nesting limit"));
        }
        let Some(token) = self.toks.get(self.at) else {
            return Err(error(self.pos(),"incomplete scalar return expression"));
        };
        let pos = token.pos;
        let tok = token.tok.clone();
        self.at += 1;
        let mut lhs = match tok {
            Tok::Int(Some(n)) if n <= i64::MAX as u64 => self.node(ScalarExpr::Const(n as i64))?,
            Tok::Int(Some(_)) | Tok::Int(None) =>
                return Err(Diagnostic::new(pos,"E_INTEGER_RANGE",
                    "signed scalar literal outside i64 range")),
            Tok::Ident(name) => {
                let param = self.receiver.params.iter().position(|p|p.name == name)
                    .ok_or_else(||error(pos,
                        "scalar preview requires a named integer parameter or literal"))?;
                self.node(ScalarExpr::Param(param as u32))?
            }
            Tok::Minus => {
                // Lexer encodes positive 2^63 as Int(None). The only valid
                // use is immediately following unary '-' to form i64::MIN.
                if matches!(self.toks.get(self.at).map(|t|&t.tok),Some(Tok::Int(None))) {
                    self.at += 1;
                    self.node(ScalarExpr::Const(i64::MIN))?
                } else {
                    let inner=self.expr(3,depth+1)?;
                    self.node(ScalarExpr::Neg(Box::new(inner)))?
                }
            }
            Tok::LParen => {
                let inner=self.expr(0,depth+1)?;
                if !matches!(self.toks.get(self.at).map(|t|&t.tok),Some(Tok::RParen)) {
                    return Err(error(self.pos(),"missing closing parenthesis in scalar preview"));
                }
                self.at+=1;
                inner
            }
            _ => return Err(error(pos,
                "scalar preview admits only integer literals, parameters, parentheses and checked arithmetic")),
        };
        loop {
            let Some(next)=self.toks.get(self.at) else {break};
            let (binding,op)=match next.tok {
                Tok::Plus => (1,0),
                Tok::Minus => (1,1),
                Tok::Star => (2,2),
                _=>break,
            };
            if binding < precedence {break}
            self.at += 1;
            let rhs=self.expr(binding+1,depth+1)?;
            lhs=self.node(match op {
                0=>ScalarExpr::Add(Box::new(lhs),Box::new(rhs)),
                1=>ScalarExpr::Sub(Box::new(lhs),Box::new(rhs)),
                _=>ScalarExpr::Mul(Box::new(lhs),Box::new(rhs)),
            })?;
        }
        Ok(lhs)
    }
}

/// Every accepted body is exactly { return <i64 expr>; }, with no hidden
/// branch, temporary, state operation, call, suspend, throw or trailing code.
pub fn lower_scalar_body(
    body: &ReceiverBodyTokens,
    receiver: &TypedReceiver,
    owner: &str,
) -> Result<ScalarMethod, Diagnostic> {
    let toks=&body.tokens;
    if toks.len() < 5 || toks.len()>1024
        || !matches!(toks.first().map(|t|&t.tok),Some(Tok::LBrace))
        || !matches!(toks.last().map(|t|&t.tok),Some(Tok::RBrace))
        || !matches!(toks.get(1).map(|t|&t.tok),Some(Tok::Ident(s)) if s=="return")
        || !matches!(toks.get(toks.len()-2).map(|t|&t.tok),Some(Tok::Semi)) {
        return Err(error(receiver.pos,
            "scalar preview requires exactly one braced return expression"));
    }
    if receiver.mutable || receiver.returns != TargetType::Int
        || receiver.params.len() > 8
        || receiver.params.iter().any(|p|p.ty!=TargetType::Int) {
        return Err(error(receiver.pos,
            "scalar preview requires immutable receiver, up to eight int parameters and int return"));
    }
    let expr_toks=&toks[2..toks.len()-2];
    let mut parser=ExprParser{toks:expr_toks,at:0,receiver,nodes:0};
    let expr=parser.expr(0,0)?;
    if parser.at != expr_toks.len() {
        return Err(error(parser.pos(),
            "unconsumed scalar tokens or statements are forbidden in executable preview"));
    }
    Ok(ScalarMethod{
        owner:owner.to_owned(),name:receiver.method.clone(),
        arity:receiver.params.len(),pos:receiver.pos,expr,
    })
}

/// Public, explicit preview entry point. The normal compile() function does
/// not call this: actor dispatch and state ownership are not implemented.
fn validate_scalar_receiver_methods(source:&str) -> Result<Vec<ScalarMethod>,Diagnostic> {
    if source.len()>crate::SOURCE_CAP {
        return Err(Diagnostic::new(Pos{line:1,col:1},"E_SOURCE_LIMIT",
            "source exceeds scalar preview byte budget"));
    }
    let toks=crate::lexer::lex(source)?;
    let headers=crate::target_syntax::parse_target_declarations(&toks)?;
    let typed=crate::target_typed::lower_target_signatures(&headers)?;
    let interface=crate::target_typed::build_actor_program(&headers,&typed);
    crate::check::check_target_actor_interface(&interface)?;
    if !headers.imports.is_empty() || !headers.definitions.is_empty()
        || headers.receivers.is_empty() || headers.receivers.len()>32
        || toks.iter().any(|t|matches!(&t.tok,Tok::Ident(s) if s=="fnc")) {
        return Err(error(Pos{line:1,col:1},
            "scalar preview needs only actor forwards and out-of-line scalar methods; no imports, state definitions or functions"));
    }
    if typed.receivers.len()!=headers.receiver_bodies.len()
        || typed.receivers.len()!=typed.receiver_cleanup.len() {
        return Err(error(Pos{line:1,col:1},"incomplete typed receiver preview"));
    }
    let mut methods=Vec::new();
    for (i,receiver) in typed.receivers.iter().enumerate() {
        let owner=typed.actors.get(receiver.actor)
            .ok_or_else(||error(receiver.pos,"invalid nominal actor in scalar preview"))?;
        // The normal frontend marks `return (expr)` as a potential call
        // because its effect scanner is deliberately conservative. The
        // opt-in preview instead proves the *entire* method expression first:
        // no calls/closures/state operations can survive lower_scalar_body.
        let method=lower_scalar_body(&headers.receiver_bodies[i],receiver,&owner.name)?;
        let effects=&receiver.effects;
        if !typed.receiver_cleanup[i].leases.is_empty() ||
            effects.reads_state || effects.writes_state ||
            effects.may_suspend || effects.may_unwind ||
            effects.may_spawn || effects.may_allocate ||
            headers.receiver_calls.iter().any(|c|c.caller==i) {
            return Err(error(receiver.pos,
                "scalar preview requires lease-free and call-free receiver body"));
        }
        methods.push(method);
    }
    Ok(methods)
}

/// Explicit opt-in stateless preview; production receiver compilation remains closed.
pub fn compile_scalar_receiver_preview(source:&str) -> Result<Vec<u8>,Diagnostic> {
    let methods=validate_scalar_receiver_methods(source)?;
    let bytes=crate::wasm::emit_scalar_receiver_preview(&methods)?;
    if bytes.len()>crate::wasm::MAX_OUTPUT {
        return Err(error(Pos{line:1,col:1},"scalar preview Wasm output cap exceeded"));
    }
    Ok(bytes)
}

/// Restricted actor-instance-bound scalar receiver preview. The generated
/// module must be bound once by a trusted host before any receiver can run.
/// This is not a scheduler, state layout or lease-cleanup implementation.
pub fn compile_scalar_receiver_instance_preview(source:&str) -> Result<Vec<u8>,Diagnostic> {
    let methods=validate_scalar_receiver_methods(source)?;
    let bytes=crate::wasm::emit_scalar_receiver_instance_preview(&methods)?;
    if bytes.len()>crate::wasm::MAX_OUTPUT {
        return Err(error(Pos{line:1,col:1},"receiver instance preview Wasm output cap exceeded"));
    }
    Ok(bytes)
}

//! Typed signature IR for proposed namespace-scoped actor receiver declarations.
//! This is separate from executable check.rs. Forward-declared actors have no
//! linked implementation, state layout, proven effects or borrow semantics.

use crate::ast::{ActorDecl, ActorInterface, ActorInterfaceMethod, FieldMode, Param, Program, Type};
use crate::diag::{Diagnostic, Pos};
use crate::target_syntax::{ReceiverEffects, ReceiverHeader, ReceiverReturn, StateAssignment, StateUse, TargetHeaders, revalidate_receiver_facts};
use crate::lexer::{Tok, Token};
use crate::target_flow::{ReceiverFlow, analyze_receiver_body, check_receiver_arguments};
use crate::target_borrow::{ReceiverBorrowFacts, check_receiver_borrows};
use crate::target_cleanup::{ReceiverCleanupPlan, plan_receiver_cleanup};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TargetType {
    Int,
    Bool,
    Float,
    Str,
    Void,
    /// Index in TypedActorModule.actors. Capability, not actor value.
    ActorRef(usize),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TypedParam {
    pub name: String,
    pub ty: TargetType,
    pub pos: Pos,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TypedActor {
    pub name: String,
    /// Concrete syntax definition linked to the forward declaration;
    /// this alone does NOT make the actor spawnable.
    pub has_definition: bool,
    pub defined: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TypedReceiver {
    pub actor: usize,
    pub method: String,
    pub arity: usize,
    pub params: Vec<TypedParam>,
    pub returns: TargetType,
    pub mutable: bool,
    pub public: bool,
    pub pos: Pos,
    /// Signatures only: method bodies are not lowered or executable.
    pub body_lowered: bool,
    /// Conservative transitive effects, inferred over nominal receiver calls.
    /// This is not an executable effect proof.
    pub effects: ReceiverEffects,
}

/// A state access validated against a concrete actor field declaration.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TypedStateUse {
    pub receiver: usize,
    pub field: String,
    pub pos: Pos,
    pub mutation: bool,
    pub exclusive: bool,
    pub numeric_mutation: bool,
    /// None means the actor is only forward-declared; no field proof exists.
    pub field_ty: Option<TargetType>,
}

/// A single syntactically complete return statement in the pre-executable
/// actor receiver IR. None means its expression needs the real body AST.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TypedBodyReturn {
    pub receiver: usize,
    pub pos: Pos,
    pub result: Option<TargetType>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TypedActorModule {
    pub namespace: Option<String>,
    pub actors: Vec<TypedActor>,
    pub receivers: Vec<TypedReceiver>,
    pub state_uses: Vec<TypedStateUse>,
    /// Partial return-type facts, not a complete control-flow or AOT proof.
    pub body_returns: Vec<TypedBodyReturn>,
    /// Structured flow facts; never an executable actor body proof.
    pub receiver_flows: Vec<ReceiverFlow>,
    /// Conservative path-sensitive ownership and lexical loan facts.
    /// These never license executable receiver lowering.
    pub receiver_borrows: Vec<ReceiverBorrowFacts>,
    /// Frontend lock-acquire/normal/trap cleanup obligations. The runtime
    /// backend has not discharged them; all out-of-line mtds remain gated.
    pub receiver_cleanup: Vec<ReceiverCleanupPlan>,
}

fn type_error(pos: Pos, why: impl Into<String>) -> Diagnostic {
    Diagnostic::new(pos, "E_TYPE", why)
}

impl TypedActorModule {
    pub fn actor_index(&self, actor: &str, pos: Pos) -> Result<usize, Diagnostic> {
        self.actors.iter().position(|a| a.name == actor)
            .ok_or_else(|| Diagnostic::new(pos, "E_UNBOUND",
                format!("undeclared actor type '{actor}'")))
    }

    /// Unambiguous in-module lookup, not actor runtime dispatch.
    pub fn resolve<'a>(&'a self, namespace: Option<&str>, actor: &str,
        method: &str, arity: usize, external: bool,
        pos: Pos) -> Result<&'a TypedReceiver, Diagnostic> {
        if namespace != self.namespace.as_deref() {
            return Err(Diagnostic::new(pos, "E_NAMESPACE",
                "receiver not in its defining namespace"));
        }
        let owner = self.actor_index(actor, pos)?;
        let method = self.receivers.iter().find(|m|
            m.actor == owner && m.method == method && m.arity == arity)
            .ok_or_else(|| Diagnostic::new(pos, "E_UNBOUND",
                format!("no receiver method {actor}.{method}/{arity}")))?;
        if external && !method.public {
            return Err(Diagnostic::new(pos, "E_VISIBILITY",
                format!("receiver method {actor}.{} is not public", method.method)));
        }
        Ok(method)
    }

    /// All source-level actors in this IR are forward declarations. Runtime
    /// linking must establish a real implementation before spawn can work.
    pub fn require_spawnable(&self, actor: &str, pos: Pos)
        -> Result<usize, Diagnostic> {
        let index = self.actor_index(actor, pos)?;
        if !self.actors[index].defined {
            return Err(Diagnostic::new(pos, "E_UNLINKED",
                format!("actor '{actor}' is forward-declared but not defined")));
        }
        Ok(index)
    }

    fn type_of(&self, name: &str, pos: Pos) -> Result<TargetType, Diagnostic> {
        Ok(match name {
            "int" => TargetType::Int,
            "bool" | "boolean" => TargetType::Bool,
            "float" | "f64" => TargetType::Float,
            "string" | "String" => TargetType::Str,
            "void" => TargetType::Void,
            _ if name.starts_with("ActorRef<") && name.ends_with('>') => {
                let actor = &name["ActorRef<".len()..name.len()-1];
                TargetType::ActorRef(self.actor_index(actor, pos)?)
            },
            _ if self.actors.iter().any(|a| a.name == name) =>
                return Err(type_error(pos,
                    "actor values cannot be passed by value; use ActorRef<Actor>")),
            _ => return Err(Diagnostic::new(pos, "E_UNKNOWN_TYPE",
                format!("unknown receiver signature type '{name}'"))),
        })
    }

    fn check_state_use(&self, h: &TargetHeaders, access: &StateUse)
        -> Result<TypedStateUse, Diagnostic> {
        let receiver = h.receivers.get(access.receiver)
            .ok_or_else(|| type_error(access.pos, "invalid receiver state access index"))?;
        // Re-check lease invariants in the typed lowering layer rather than
        // trusting potentially constructed/imported syntax headers.
        if access.exclusive && !receiver.mutable {
            return Err(Diagnostic::new(access.pos, "E_MUTABILITY",
                "critical-write requires a mutable actor receiver"));
        }
        if access.mutation && !access.exclusive {
            return Err(Diagnostic::new(access.pos, "E_READONLY",
                "critical-read cannot mutate actor state"));
        }
        let field_ty = if let Some(actor) = h.definitions.iter()
            .find(|a| a.name == receiver.owner) {
            let field = actor.fields.iter().find(|f| f.name == access.field)
                .ok_or_else(|| Diagnostic::new(access.pos, "E_UNBOUND",
                    format!("actor '{}': unknown state field '{}'",
                        actor.name, access.field)))?;
            if access.mutation && field.mode != FieldMode::ActorState {
                return Err(Diagnostic::new(access.pos, "E_READONLY",
                    format!("actor state field '{}.{}' is immutable",
                        actor.name, access.field)));
            }
            if access.numeric_mutation && !matches!(field.ty, Type::Int | Type::Float) {
                return Err(type_error(access.pos,
                    "increment/decrement and compound addition require a numeric state field"));
            }
            // Only by-value scalar state projections can currently be proved
            // not to leak the actor-owned heap. Strings, handles, containers,
            // and named reference types require a real typed borrow checker.
            Some(match field.ty {
                Type::Int => TargetType::Int,
                Type::Bool => TargetType::Bool,
                Type::Float => TargetType::Float,
                _ => return Err(Diagnostic::new(access.pos, "E_BORROW",
                    format!("state field '{}.{}' requires ownership-safe typed projection",
                        actor.name, access.field))),
            })
        } else {
            // There is no state layout to prove. Keep an explicit unverified
            // fact; the target cannot emit actor code in this situation.
            None
        };
        Ok(TypedStateUse {
            receiver: access.receiver, field: access.field.clone(), pos: access.pos,
            mutation: access.mutation, exclusive: access.exclusive,
            numeric_mutation: access.numeric_mutation, field_ty,
        })
    }

    fn check_scalar_assignment(&self, headers: &TargetHeaders, a: &StateAssignment)
        -> Result<(), Diagnostic> {
        let receiver = headers.receivers.get(a.receiver)
            .ok_or_else(|| type_error(a.pos, "invalid state assignment receiver"))?;
        // A forward-only actor has no state layout to verify: keep it
        // explicitly unverified and unexecutable.
        let Some(actor) = headers.definitions.iter().find(|a| a.name == receiver.owner)
            else { return Ok(()); };
        let expected = scalar_field_type(actor, &a.field, a.pos)?;
        if a.compound && !matches!(expected, TargetType::Int | TargetType::Float) {
            return Err(type_error(a.pos, "compound state addition requires a numeric field"));
        }
        let mut parser = ScalarParser {
            tokens: &a.rhs, index: 0, binder: &a.binder,
            owner: actor, receiver, module: self,
        };
        let actual = parser.expr(0, 0)?;
        if parser.index != parser.tokens.len() {
            return Err(Diagnostic::new(parser.tokens[parser.index].pos, "E_UNSUPPORTED",
                "state assignment expression requires fully typed scalar lowering"));
        }
        if actual != expected {
            return Err(type_error(a.pos,
                format!("state field '{}.{}' expects {:?}, got {:?}",
                    actor.name, a.field, expected, actual)));
        }
        Ok(())
    }

    /// Only type expressions whose complete meaning is statically known.
    /// All other return bodies remain unproved and explicitly non-executable.
    fn check_receiver_return(&self, headers: &TargetHeaders, ret: &ReceiverReturn)
        -> Result<TypedBodyReturn, Diagnostic> {
        let method = self.receivers.get(ret.receiver)
            .ok_or_else(|| type_error(ret.pos, "invalid return receiver index"))?;
        let h = headers.receivers.get(ret.receiver)
            .ok_or_else(|| type_error(ret.pos, "missing source receiver header"))?;
        let ts = &ret.expr;
        let result = match ts.as_slice() {
            [] => Some(TargetType::Void),
            [Token {tok: Tok::Int(Some(_)), ..}] => Some(TargetType::Int),
            [Token {tok: Tok::Int(None), ..}] => {
                return Err(Diagnostic::new(ret.pos, "E_INTEGER_RANGE",
                    "integer literal 9223372036854775808 requires a leading minus"));
            }
            [Token {tok: Tok::Float(_), ..}] => Some(TargetType::Float),
            [Token {tok: Tok::Str(_), ..}] => Some(TargetType::Str),
            [Token {tok: Tok::Ident(name), ..}] if name == "true" || name == "false" =>
                Some(TargetType::Bool),
            [Token {tok: Tok::Ident(name), ..}] => {
                if name == "self" {
                    return Err(Diagnostic::new(ret.pos, "E_ESCAPE",
                        "actor receiver cannot escape by return"));
                }
                h.params.iter().find(|p| p.name == *name)
                    .map(|p| self.type_of(&p.type_name, p.pos))
                    .transpose()?
            }
            [Token {tok: Tok::Minus, ..}, Token {tok: Tok::Int(_), ..}] =>
                Some(TargetType::Int),
            // A zero-argument nominal self call has a known return signature.
            [Token {tok: Tok::Ident(owner), ..}, Token {tok: Tok::Dot, ..},
             Token {tok: Tok::Ident(name), ..}, Token {tok: Tok::LParen, ..},
             Token {tok: Tok::RParen, ..}] if owner == "self" && name != "handle_state" => {
                Some(self.resolve(headers.namespace.as_deref(), &h.owner,
                    name, 0, false, ret.pos)?.returns.clone())
            }
            // By-value scalar projection from a concrete actor under a
            // critical-read lease. No reference/handle is permitted to escape.
            [Token {tok: Tok::Ident(owner), ..}, Token {tok: Tok::Dot, ..},
             Token {tok: Tok::Ident(access), ..}, Token {tok: Tok::LParen, ..},
             Token {tok: Tok::Ident(critical), ..}, Token {tok: Tok::Minus, ..},
             Token {tok: Tok::Ident(mode), ..}, Token {tok: Tok::Pipe, ..},
             Token {tok: Tok::Ident(binder), ..}, Token {tok: Tok::Pipe, ..},
             Token {tok: Tok::Arrow, ..}, Token {tok: Tok::Ident(value), ..},
             Token {tok: Tok::Dot, ..}, Token {tok: Tok::Ident(field), ..},
             Token {tok: Tok::RParen, ..}]
              if owner == "self" && access == "handle_state" && critical == "critical"
                  && (mode == "read" || mode == "write") && binder == value => {
                headers.definitions.iter().find(|a| a.name == h.owner)
                    .map(|a| scalar_field_type(a, field, ret.pos))
                    .transpose()?
            }
            _ => None,
        };
        if let Some(actual) = &result {
            if *actual != method.returns {
                return Err(type_error(ret.pos, format!(
                    "receiver '{}.{}' returns {:?}, but return expression has {:?}",
                    h.owner, h.name, method.returns, actual)));
            }
        }
        Ok(TypedBodyReturn { receiver: ret.receiver, pos: ret.pos, result })
    }

    fn lower_one(&self, h: &ReceiverHeader) -> Result<TypedReceiver, Diagnostic> {
        // Sealed lifecycle hooks belong to the actor runtime. This invariant
        // applies even to forward-only actors with no concrete definition.
        if h.name == "receive" {
            return Err(Diagnostic::new(h.pos, "E_SIGNATURE",
                "receive is a sealed actor lifecycle handler"));
        }
        let actor = self.actor_index(&h.owner, h.pos)?;
        let mut params = Vec::with_capacity(h.params.len());
        for p in &h.params {
            let ty = self.type_of(&p.type_name, p.pos)?;
            if ty == TargetType::Void {
                return Err(type_error(p.pos, "a method parameter cannot have type void"));
            }
            params.push(TypedParam { name: p.name.clone(), ty, pos: p.pos });
        }
        let returns = self.type_of(&h.return_type, h.pos)?;
        if h.arity != params.len() {
            return Err(type_error(h.pos, "internal receiver arity mismatch"));
        }
        Ok(TypedReceiver {
            actor, method: h.name.clone(), arity: params.len(), params, returns,
            mutable: h.mutable, public: h.public, pos: h.pos,
            body_lowered: false,
            effects: ReceiverEffects::default(),
        })
    }
}

/// The scalar-only lease expression checker deliberately admits no calls,
/// references, arrays, closures or allocation. More complex expressions must
/// wait for the production borrow/effect checker before executable lowering.
fn scalar_field_type(actor: &ActorDecl, name: &str, pos: Pos)
    -> Result<TargetType, Diagnostic> {
    let field = actor.fields.iter().find(|field| field.name == name)
        .ok_or_else(|| Diagnostic::new(pos, "E_UNBOUND",
            format!("actor '{}' has no state field '{}'", actor.name, name)))?;
    match field.ty {
        Type::Int => Ok(TargetType::Int),
        Type::Bool => Ok(TargetType::Bool),
        Type::Float => Ok(TargetType::Float),
        _ => Err(Diagnostic::new(pos, "E_BORROW",
            "non-scalar actor-owned values need typed borrow and lifetime proofs")),
    }
}

struct ScalarParser<'a> {
    tokens: &'a [Token],
    index: usize,
    binder: &'a str,
    owner: &'a ActorDecl,
    receiver: &'a ReceiverHeader,
    module: &'a TypedActorModule,
}

impl ScalarParser<'_> {
    fn current_pos(&self) -> Pos {
        self.tokens.get(self.index).map_or(Pos::default(), |token| token.pos)
    }
    fn expr(&mut self, min_prec: u8, depth: usize) -> Result<TargetType, Diagnostic> {
        if depth > 128 {
            return Err(Diagnostic::new(self.current_pos(), "E_NESTING",
                "scalar actor expression nesting limit"));
        }
        let token = self.tokens.get(self.index)
            .ok_or_else(|| Diagnostic::new(self.current_pos(), "E_SYNTAX",
                "expected scalar state expression"))?;
        let pos = token.pos;
        let kind = token.tok.clone();
        self.index += 1;
        let mut lhs = match kind {
            // The lexer represents exactly 2^63 as Int(None). It is only
            // legal when immediately negated, producing i64::MIN.
            Tok::Int(Some(_)) => TargetType::Int,
            Tok::Int(None) => return Err(Diagnostic::new(pos, "E_INTEGER_RANGE",
                "integer literal 9223372036854775808 requires a leading unary minus")),
            Tok::Float(_) => TargetType::Float,
            Tok::Str(_) => TargetType::Str,
            Tok::Ident(name) if name == "true" || name == "false" => TargetType::Bool,
            Tok::Ident(name) if name == self.binder => {
                if !matches!(self.tokens.get(self.index).map(|t| &t.tok), Some(Tok::Dot)) {
                    return Err(Diagnostic::new(pos, "E_ESCAPE",
                        "the state binder cannot escape by value"));
                }
                self.index += 1;
                let member = self.tokens.get(self.index)
                    .ok_or_else(|| Diagnostic::new(pos, "E_SYNTAX", "missing state field"))?;
                let Tok::Ident(field) = &member.tok else {
                    return Err(Diagnostic::new(member.pos, "E_SYNTAX", "expected state field name"));
                };
                self.index += 1;
                scalar_field_type(self.owner, field, member.pos)?
            }
            Tok::Ident(name) => {
                let param = self.receiver.params.iter().find(|p| p.name == name)
                    .ok_or_else(|| Diagnostic::new(pos, "E_UNBOUND",
                        format!("unknown scalar expression binding '{name}'")))?;
                self.module.type_of(&param.type_name, param.pos)?
            }
            Tok::Minus => {
                // Parse -9223372036854775808 as a single checked value, not
                // an out-of-range positive integer followed by negation.
                if matches!(self.tokens.get(self.index).map(|t| &t.tok),
                    Some(Tok::Int(None))) {
                    self.index += 1;
                    TargetType::Int
                } else {
                    let inner = self.expr(5, depth + 1)?;
                    if !matches!(inner, TargetType::Int | TargetType::Float) {
                        return Err(type_error(pos, "unary minus requires a numeric scalar"));
                    }
                    inner
                }
            }
            Tok::Bang => {
                let inner = self.expr(5, depth + 1)?;
                if inner != TargetType::Bool {
                    return Err(type_error(pos, "logical negation requires a bool"));
                }
                TargetType::Bool
            }
            Tok::LParen => {
                let inner = self.expr(0, depth + 1)?;
                if !matches!(self.tokens.get(self.index).map(|t| &t.tok), Some(Tok::RParen)) {
                    return Err(Diagnostic::new(self.current_pos(), "E_SYNTAX",
                        "unclosed scalar expression group"));
                }
                self.index += 1;
                inner
            }
            _ => return Err(Diagnostic::new(pos, "E_UNSUPPORTED",
                "unproved scalar state expression requires typed lowering")),
        };
        loop {
            let Some(next) = self.tokens.get(self.index) else { break };
            let (prec, equality, relational) = match &next.tok {
                Tok::EqEq | Tok::NotEq => (1, true, false),
                Tok::Ident(word) if word == "eq" || word == "neq" => (1, true, false),
                Tok::Lt | Tok::Le | Tok::Gt | Tok::Ge => (2, false, true),
                Tok::Plus | Tok::Minus => (3, false, false),
                Tok::Star | Tok::Slash => (4, false, false),
                _ => break,
            };
            if prec < min_prec { break; }
            let op_pos = next.pos;
            self.index += 1;
            let rhs = self.expr(prec + 1, depth + 1)?;
            if lhs != rhs {
                return Err(type_error(op_pos, "scalar operands have incompatible types"));
            }
            if equality || relational {
                if relational && !matches!(lhs, TargetType::Int | TargetType::Float) {
                    return Err(type_error(op_pos, "ordered comparison requires numeric scalars"));
                }
                if equality && !matches!(lhs, TargetType::Int | TargetType::Float | TargetType::Bool) {
                    return Err(type_error(op_pos, "equality requires comparable scalars"));
                }
                lhs = TargetType::Bool;
            } else if !matches!(lhs, TargetType::Int | TargetType::Float) {
                return Err(type_error(op_pos, "arithmetic requires numeric scalars"));
            }
        }
        Ok(lhs)
    }
}

/// Resolve the entire declaration set before typing receiver parameters and
/// result annotations, so forward references within one file are permitted.
pub fn lower_target_signatures(headers: &TargetHeaders)
    -> Result<TypedActorModule, Diagnostic> {
    // Imported headers can omit source-derived state writes, reads, returns,
    // or calls; re-derive the complete facts before any typed proof is used.
    revalidate_receiver_facts(headers)?;
    let mut module = TypedActorModule {
        namespace: headers.namespace.clone(),
        actors: headers.actors.iter().map(|name| TypedActor {
            name: name.clone(),
            has_definition: headers.definitions.iter().any(|a| a.name == *name),
            defined: false,
        }).collect(),
        receivers: Vec::with_capacity(headers.receivers.len()),
        state_uses: Vec::with_capacity(headers.state_uses.len()),
        body_returns: Vec::with_capacity(headers.receiver_returns.len()),
        receiver_flows: Vec::with_capacity(headers.receiver_bodies.len()),
        receiver_borrows: Vec::with_capacity(headers.receiver_bodies.len()),
        receiver_cleanup: Vec::with_capacity(headers.receiver_bodies.len()),
    };
    for definition in &headers.definitions {
        if !headers.actors.contains(&definition.name) {
            return Err(Diagnostic::new(definition.pos, "E_UNBOUND",
                format!("concrete actor '{}' has no matching declare actor", definition.name)));
        }
    }
    for header in &headers.receivers {
        module.receivers.push(module.lower_one(header)?);
    }
    for access in &headers.state_uses {
        module.state_uses.push(module.check_state_use(headers, access)?);
    }
    for assignment in &headers.state_assignments {
        module.check_scalar_assignment(headers, assignment)?;
    }
    for ret in &headers.receiver_returns {
        module.body_returns.push(module.check_receiver_return(headers, ret)?);
    }
    if headers.receiver_effects.len() != module.receivers.len() {
        return Err(type_error(Pos::default(),
            "missing receiver effect facts; typed lowering requires full body preflight"));
    }
    for (receiver, direct) in module.receivers.iter_mut().zip(&headers.receiver_effects) {
        receiver.effects = direct.clone();
    }
    // Known argument types are checked even when the caller's body cannot
    // yet be lowered. Unknown expressions explicitly preserve an unknown
    // effect obligation rather than silently counting as safe.
    let unknown_args = check_receiver_arguments(headers, &module)?;
    for (receiver, unknown) in module.receivers.iter_mut().zip(unknown_args) {
        receiver.effects.unproved_call |= unknown;
    }
    // Imported TargetHeaders may be forged or truncated. Every receiver
    // method must have exactly one complete body, in nominal declaration
    // order. A missing body is NOT evidence of a lease-free receiver.
    if headers.receiver_bodies.len() != module.receivers.len() {
        return Err(Diagnostic::new(Pos::default(),"E_SEALED",
            "missing receiver body prevents validating state-lease cleanup"));
    }
    for (index, body) in headers.receiver_bodies.iter().enumerate() {
        if body.receiver != index {
            return Err(Diagnostic::new(
                headers.receivers[index].pos,"E_SEALED",
                "receiver body index mismatch prevents cleanup validation"));
        }
        module.receiver_flows.push(analyze_receiver_body(headers, &module, body)?);
        module.receiver_borrows.push(check_receiver_borrows(&module, body)?);
        let cleanup = plan_receiver_cleanup(body)?;
        // Independently cross-check syntax effect facts against the actual
        // source-located lease regions. Calls' *transitive* state effects
        // are deliberately excluded: only direct callback acquisitions count.
        let direct = &headers.receiver_effects[index];
        let has_lease = !cleanup.leases.is_empty();
        let has_write_lease = cleanup.leases.iter().any(|lease|
            matches!(lease.mode,crate::target_cleanup::StateLeaseMode::ExclusiveWrite));
        if direct.reads_state != has_lease || direct.writes_state != has_write_lease {
            return Err(Diagnostic::new(
                headers.receivers[index].pos,"E_SEALED",
                "state-effect facts disagree with required lease cleanup regions"));
        }
        module.receiver_cleanup.push(cleanup);
    }
    // Bounded monotone fixed point: each effect bit can transition only from
    // false to true. Recursive calls are handled without unbounded DFS.
    for _ in 0..module.receivers.len() {
        let mut changed = false;
        for call in &headers.receiver_calls {
            let callee = module.receivers.get(call.callee)
                .ok_or_else(|| type_error(call.pos, "invalid receiver call target"))?;
            let target_actor = callee.actor;
            let transitive = callee.effects.clone();
            let caller = module.receivers.get_mut(call.caller)
                .ok_or_else(|| type_error(call.pos, "invalid receiver caller"))?;
            if caller.actor != target_actor {
                return Err(Diagnostic::new(call.pos, "E_ISOLATION",
                    "receiver calls cannot cross nominal actor boundaries"));
            }
            let before = caller.effects.clone();
            caller.effects.reads_state |= transitive.reads_state;
            caller.effects.writes_state |= transitive.writes_state;
            caller.effects.may_suspend |= transitive.may_suspend;
            caller.effects.may_unwind |= transitive.may_unwind;
            caller.effects.may_panic |= transitive.may_panic;
            caller.effects.may_spawn |= transitive.may_spawn;
            caller.effects.may_allocate |= transitive.may_allocate;
            caller.effects.unproved_call |= transitive.unproved_call;
            changed |= caller.effects != before;
        }
        if !changed { break; }
    }
    Ok(module)
}


impl TargetType {
    /// ActorRef indices are into actor_interface.forwards, not runtime actors.
    pub fn as_ast_type(&self) -> Type {
        match *self {
            Self::Int => Type::Int,
            Self::Bool => Type::Bool,
            Self::Float => Type::Float,
            Self::Str => Type::Str,
            Self::Void => Type::Void,
            Self::ActorRef(index) => Type::ActorRef(index as u32),
        }
    }
}

/// Preserve real concrete ActorDecls and nominal receiver signatures inside
/// the production AST, for checking before any actor code generation.
pub fn build_actor_program(headers: &TargetHeaders, module: &TypedActorModule) -> Program {
    let interface = ActorInterface {
        namespace: module.namespace.clone(),
        forwards: module.actors.iter().map(|a| a.name.clone()).collect(),
        methods: module.receivers.iter().map(|m| ActorInterfaceMethod {
            owner: m.actor as u32,
            name: m.method.clone(),
            params: m.params.iter().map(|p| Param {
                name: p.name.clone(), ty: p.ty.as_ast_type(),
                mutable: false, pos: p.pos,
            }).collect(),
            ret: m.returns.as_ast_type(),
            mutable: m.mutable, public: m.public, pos: m.pos,
        }).collect(),
    };
    Program {
        actors: headers.definitions.clone(),
        actor_interface: Some(interface),
        ..Program::default()
    }
}

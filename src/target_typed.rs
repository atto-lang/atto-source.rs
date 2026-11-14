//! Typed signature IR for proposed namespace-scoped actor receiver declarations.
//! This is separate from executable check.rs. Forward-declared actors have no
//! linked implementation, state layout, proven effects or borrow semantics.

use crate::ast::{ActorInterface, ActorInterfaceMethod, FieldMode, Param, Program, Type};
use crate::diag::{Diagnostic, Pos};
use crate::target_syntax::{ReceiverEffects, ReceiverHeader, StateUse, TargetHeaders};

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
    /// None means the actor is only forward-declared; no field proof exists.
    pub field_ty: Option<TargetType>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TypedActorModule {
    pub namespace: Option<String>,
    pub actors: Vec<TypedActor>,
    pub receivers: Vec<TypedReceiver>,
    pub state_uses: Vec<TypedStateUse>,
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
            mutation: access.mutation, exclusive: access.exclusive, field_ty,
        })
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

/// Resolve the entire declaration set before typing receiver parameters and
/// result annotations, so forward references within one file are permitted.
pub fn lower_target_signatures(headers: &TargetHeaders)
    -> Result<TypedActorModule, Diagnostic> {
    let mut module = TypedActorModule {
        namespace: headers.namespace.clone(),
        actors: headers.actors.iter().map(|name| TypedActor {
            name: name.clone(),
            has_definition: headers.definitions.iter().any(|a| a.name == *name),
            defined: false,
        }).collect(),
        receivers: Vec::with_capacity(headers.receivers.len()),
        state_uses: Vec::with_capacity(headers.state_uses.len()),
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
    if headers.receiver_effects.len() != module.receivers.len() {
        return Err(type_error(Pos::default(),
            "missing receiver effect facts; typed lowering requires full body preflight"));
    }
    for (receiver, direct) in module.receivers.iter_mut().zip(&headers.receiver_effects) {
        receiver.effects = direct.clone();
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

//! Typed signature IR for proposed namespace-scoped actor receiver declarations.
//! This is separate from executable check.rs. Forward-declared actors have no
//! linked implementation, state layout, proven effects or borrow semantics.

use crate::ast::{ActorInterface, ActorInterfaceMailboxIngress, ActorInterfaceMethod, ActorInterfacePort, Param, Program, Type};
use crate::diag::{Diagnostic, Pos};
use crate::target_syntax::{ReceiverHeader, TargetHeaders};

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
    pub public: bool,
    pub receives: Option<String>,
    pub emits: Option<String>,
    pub sends: Option<String>,
    pub implemented_contracts: Vec<String>,
    /// Exactly one matching inline or attached receive(ActorMail<T>) was
    /// admitted by the static namespace linker. This is NOT runtime authority.
    pub inbox_witnessed: bool,
    /// An emitted-output effect needs a separate checked implementation.
    /// No target-only token scan can certify an emit effect.
    pub emits_witnessed: bool,
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
}

/// Private runtime ingress signature attached to a declared actor.
/// Never resolved as a normal receiver method and never guest executable here.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TypedMailboxIngress {
    pub actor: usize,
    pub payload: String,
    pub pos: Pos,
    pub body_lowered: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TypedActorModule {
    pub namespace: Option<String>,
    pub actors: Vec<TypedActor>,
    pub receivers: Vec<TypedReceiver>,
    pub mailbox_ingresses: Vec<TypedMailboxIngress>,
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

    fn lower_one(&self, h: &ReceiverHeader) -> Result<TypedReceiver, Diagnostic> {
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
        })
    }
}

/// Resolve the entire declaration set before typing receiver parameters and
/// result annotations, so forward references within one file are permitted.
pub fn lower_target_signatures(headers: &TargetHeaders)
    -> Result<TypedActorModule, Diagnostic> {
    let mut module = TypedActorModule {
        namespace: headers.namespace.clone(),
        actors: headers.actor_ports.iter().map(|port| TypedActor {
            name: port.name.clone(),
            public: port.public,
            receives: port.receives.clone(),
            emits: port.emits.clone(),
            sends: port.sends.clone(),
            implemented_contracts: port.implements.clone(),
            inbox_witnessed: false,
            emits_witnessed: false,
            has_definition: headers.definitions.iter().any(|a| a.name == port.name),
            defined: false,
        }).collect(),
        receivers: Vec::with_capacity(headers.receivers.len()),
        mailbox_ingresses: Vec::new(),
    };
    for definition in &headers.definitions {
        let Some(header) = headers.actor_ports.iter().find(|h| h.name == definition.name) else {
            return Err(Diagnostic::new(definition.pos, "E_UNBOUND",
                format!("concrete actor '{}' has no matching declare actor", definition.name)));
        };
        // A header is not a substitute for the mailbox handler. Check all
        // three sources of truth: nominal declaration, concrete header,
        // and the actual typed receive handler. Never select the first one
        // and silently mask a contradictory receive body.
        if let Some(inline_input) = definition.declared_receives {
            match definition.receive.as_ref() {
                Some(receive) if receive.payload == inline_input => {}
                _ => return Err(Diagnostic::new(definition.pos, "E_SIGNATURE",
                    format!("actor '{}' concrete receives header does not match its receive(ActorMail<T>) handler",
                        definition.name))),
            }
        }
        if header.receives.is_some() && definition.receive.is_none() &&
            !headers.receivers.iter().any(|r|
                r.owner == definition.name && r.name == "receive"
            ) {
            return Err(Diagnostic::new(definition.pos, "E_SIGNATURE",
                format!("actor '{}' declared receives but has no inline or attached receive(ActorMail<T>) handler",
                    definition.name)));
        }
        // Match available primitive input/output/effect header types without
        // inventing a runtime witness for arbitrary unresolved nominal types.
        let actual_receive = definition.receive.as_ref().map(|r| r.payload);
        for (kind, declared, concrete) in [
            ("receives", header.receives.as_deref(), actual_receive),
            ("emits", header.emits.as_deref(), definition.declared_emits),
            ("sends", header.sends.as_deref(), definition.declared_sends),
        ] {
            if let (Some(declared), Some(concrete)) = (declared, concrete) {
                let actual = match concrete {
                    Type::Int => "int",
                    Type::Bool => "bool",
                    Type::Float => "float",
                    Type::Str => "string",
                    Type::Void => "void",
                    _ => return Err(Diagnostic::new(definition.pos, "E_UNSUPPORTED_TYPE",
                        format!("cannot link unresolved {kind} type for actor '{}' without a verified namespace type graph",
                            definition.name))),
                };
                let declared = match declared {
                    "boolean" => "bool",
                    "f64" => "float",
                    "String" => "string",
                    other => other,
                };
                if actual != declared {
                    return Err(Diagnostic::new(definition.pos, "E_SIGNATURE",
                        format!("actor '{}' {kind} contract mismatch: declaration {declared}, definition {actual}",
                            definition.name)));
                }
            }
        }
    }
    for header in &headers.receivers {
        if header.name != "receive" {
            if header.params.iter().any(|p| p.type_name.starts_with("ActorMail<")) {
                return Err(Diagnostic::new(header.pos, "E_SIGNATURE",
                    "ActorMail<T> is sealed to a private actor receive handler"));
            }
            module.receivers.push(module.lower_one(header)?);
            continue;
        }
        // A method called receive on a class is ordinary (not an actor ingress).
        if !headers.actors.contains(&header.owner) {
            module.receivers.push(module.lower_one(header)?);
            continue;
        }
        let actor = module.actor_index(&header.owner, header.pos)?;
        if header.public || header.mutable || header.arity != 1 ||
            header.return_type != "void" {
            return Err(Diagnostic::new(header.pos, "E_SIGNATURE",
                "actor receive ingress must be private, non-mutating, void and accept exactly one ActorMail<T>"));
        }
        let Some(param) = header.params.first() else {
            return Err(Diagnostic::new(header.pos, "E_SIGNATURE",
                "actor receive requires one ActorMail<T> parameter"));
        };
        let Some(payload) = param.type_name.strip_prefix("ActorMail<")
            .and_then(|t| t.strip_suffix('>')) else {
            return Err(Diagnostic::new(header.pos, "E_SIGNATURE",
                "actor receive requires exactly one ActorMail<T> parameter"));
        };
        fn normalize(name: &str) -> &str {
            match name {
                "boolean" => "bool", "String" => "string", "f64" => "float",
                other => other,
            }
        }
        if let Some(expected) = module.actors[actor].receives.as_deref() {
            if normalize(expected) != normalize(payload) {
                return Err(Diagnostic::new(header.pos, "E_SIGNATURE",
                    format!("actor '{}' receives {expected} but attached receive(ActorMail<{payload}>) disagrees",
                        header.owner)));
            }
        }
        if headers.definitions.iter().any(|d| d.name == header.owner && d.receive.is_some()) {
            return Err(Diagnostic::new(header.pos, "E_DUPLICATE",
                format!("actor '{}' has both an inline receive handler and an attached mtd receive", header.owner)));
        }
        if module.mailbox_ingresses.iter().any(|i| i.actor == actor) {
            return Err(Diagnostic::new(header.pos, "E_DUPLICATE",
                format!("duplicate private receive handler for actor '{}'", header.owner)));
        }
        module.mailbox_ingresses.push(TypedMailboxIngress {
            actor, payload: payload.to_owned(), pos: header.pos, body_lowered: false,
        });
    }
    for (i, actor) in module.actors.iter_mut().enumerate() {
        actor.inbox_witnessed = module.mailbox_ingresses.iter().any(|m| m.actor == i) ||
            headers.definitions.iter().any(|d| d.name == actor.name && d.receive.is_some());
        // Even a syntactically present emit() does not prove its type, effect,
        // backpressure or ownership semantics. Do not infer a pipe source.
        actor.emits_witnessed = false;
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
        ports: module.actors.iter().enumerate().map(|(i, a)| ActorInterfacePort {
            actor: i as u32,
            public: a.public,
            receives: a.receives.clone(),
            emits: a.emits.clone(),
            sends: a.sends.clone(),
            implemented_contracts: a.implemented_contracts.clone(),
        }).collect(),
        mailbox_ingresses: module.mailbox_ingresses.iter().map(|i| ActorInterfaceMailboxIngress {
            owner: i.actor as u32, payload: i.payload.clone(), pos: i.pos,
        }).collect(),
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

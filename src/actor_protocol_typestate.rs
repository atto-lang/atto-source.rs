//! AttoLang actor-protocol source preflight (static only; NO guest runtime).
//!
//! Protocols refine ordinary actors, never create another actor kind. This pass
//! runs before actor / mailbox lowering and never emits a fake Wasm result.
//! The deliberately narrow flow subset proves sequential affine handles; all
//! other protocol source fails closed until CFG/borrow/runtime integration.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use crate::diag::{Diagnostic, Pos};
use crate::lexer::{Tok, Token};

type R<T> = Result<T, Diagnostic>;
const MAX_STATES: usize = 64;
const MAX_TRANSITIONS: usize = 256;
const MAX_STEPS: usize = 512;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Response {
    Unary(String),
    Stream(String),
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Transition {
    pub from: String,
    pub message: String,
    pub to: String,
    pub response: Response,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Protocol {
    pub name: String,
    pub initial: String,
    pub states: BTreeSet<String>,
    pub terminal: BTreeSet<String>,
    pub transitions: Vec<Transition>,
    pub pos: Pos,
}
#[derive(Clone, Debug, Default)]
pub struct ProtocolIndex {
    pub protocols: BTreeMap<String, Protocol>,
    pub actors: BTreeMap<String, String>,
    pub completed_handles: usize,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SendProof {
    pub response: Response,
    pub next_states: Vec<String>,
}

fn at(t: &[Token], i: usize) -> Option<&Tok> { t.get(i).map(|v| &v.tok) }
fn word(t: &[Token], i: usize, w: &str) -> bool {
    matches!(at(t, i), Some(Tok::Ident(v)) if v == w)
}
fn err(t: &[Token], i: usize, code: &'static str, message: impl Into<String>) -> Diagnostic {
    Diagnostic::new(t.get(i).map_or(Pos::default(), |v| v.pos), code, message)
}
fn name(t: &[Token], i: &mut usize) -> R<String> {
    match at(t, *i) {
        Some(Tok::Ident(v)) if v.len() <= 96 => {
            *i += 1;
            Ok(v.clone())
        }
        _ => Err(err(t, *i, "E_PROTOCOL_SYNTAX", "expected protocol identifier")),
    }
}
fn require_word(t: &[Token], i: &mut usize, expected: &str) -> R<()> {
    if word(t, *i, expected) { *i += 1; Ok(()) }
    else { Err(err(t, *i, "E_PROTOCOL_SYNTAX", format!("expected '{expected}'"))) }
}
fn require_tok(t: &[Token], i: &mut usize, expected: Tok) -> R<()> {
    if at(t, *i) == Some(&expected) { *i += 1; Ok(()) }
    else { Err(err(t, *i, "E_PROTOCOL_SYNTAX", "missing protocol delimiter")) }
}
fn group_end(t: &[Token], open: usize) -> R<usize> {
    if at(t, open) != Some(&Tok::LParen) {
        return Err(err(t, open, "E_PROTOCOL_SYNTAX", "expected '('"));
    }
    let mut depth = 0_usize;
    for (i, token) in t.iter().enumerate().skip(open) {
        match token.tok {
            Tok::LParen => { depth += 1; if depth > 64 {
                return Err(err(t, i, "E_PROTOCOL_BUDGET", "call nesting exceeds budget"));
            }}
            Tok::RParen => { depth -= 1; if depth == 0 { return Ok(i); } }
            Tok::Eof => break,
            _ => {}
        }
    }
    Err(err(t, open, "E_PROTOCOL_SYNTAX", "unterminated call"))
}
fn parse_protocol(t: &[Token], start: usize) -> R<(Protocol, usize)> {
    let mut i = start;
    if word(t, i, "pub") { i += 1; }
    require_word(t, &mut i, "define")?;
    if word(t, i, "pub") { i += 1; }
    require_word(t, &mut i, "protocol")?;
    let protocol_name = name(t, &mut i)?;
    require_word(t, &mut i, "as")?;
    let mut initial = None;
    let mut states = BTreeSet::new();
    let mut terminal = BTreeSet::new();
    let mut transitions = Vec::new();
    while i < t.len() && !word(t, i, "end") {
        if word(t, i, "initial") {
            let pos = i; i += 1;
            let state = name(t, &mut i)?;
            if initial.replace(state).is_some() {
                return Err(err(t, pos, "E_PROTOCOL_DUPLICATE", "duplicate initial state"));
            }
        } else if word(t, i, "terminal") {
            let pos = i; i += 1;
            require_word(t, &mut i, "state")?;
            let state = name(t, &mut i)?;
            if !terminal.insert(state.clone()) {
                return Err(err(t, pos, "E_PROTOCOL_DUPLICATE", "duplicate terminal state"));
            }
            states.insert(state);
            if states.len() > MAX_STATES {
                return Err(err(t, pos, "E_PROTOCOL_BUDGET", "too many protocol states"));
            }
        } else if word(t, i, "state") {
            i += 1;
            let state = name(t, &mut i)?;
            if !states.insert(state.clone()) {
                return Err(err(t, i, "E_PROTOCOL_DUPLICATE", "duplicate protocol state"));
            }
            if states.len() > MAX_STATES {
                return Err(err(t, i, "E_PROTOCOL_BUDGET", "too many protocol states"));
            }
            require_word(t, &mut i, "as")?;
            while i < t.len() && !word(t, i, "end") {
                require_word(t, &mut i, "on")?;
                let message = name(t, &mut i)?;
                if at(t, i) == Some(&Tok::LParen) {
                    i = group_end(t, i)? + 1;
                }
                require_tok(t, &mut i, Tok::Arrow)?;
                let to = name(t, &mut i)?;
                let response = if word(t, i, "replies") {
                    i += 1;
                    Response::Unary(name(t, &mut i)?)
                } else if word(t, i, "streams") {
                    i += 1;
                    Response::Stream(name(t, &mut i)?)
                } else {
                    return Err(err(t, i, "E_PROTOCOL_REPLY_REQUIRED",
                        "transition must declare replies or streams"));
                };
                if transitions.iter().any(|r: &Transition| r.from == state && r.message == message) {
                    return Err(err(t, i, "E_PROTOCOL_DUPLICATE",
                        "duplicate state/message transition"));
                }
                transitions.push(Transition {from:state.clone(), message, to, response});
                if transitions.len() > MAX_TRANSITIONS {
                    return Err(err(t, i, "E_PROTOCOL_BUDGET", "too many transitions"));
                }
                if at(t, i) == Some(&Tok::Semi) { i += 1; }
            }
            require_word(t, &mut i, "end")?;
        } else {
            return Err(err(t, i, "E_PROTOCOL_SYNTAX", "invalid item in protocol declaration"));
        }
        if at(t, i) == Some(&Tok::Semi) { i += 1; }
    }
    require_word(t, &mut i, "end")?;
    if at(t, i) == Some(&Tok::Semi) { i += 1; }
    let initial = initial.ok_or_else(|| err(t, start, "E_PROTOCOL_SYNTAX", "missing initial state"))?;
    let protocol = Protocol {
        name:protocol_name, initial, states, terminal, transitions,
        pos:t[start].pos
    };
    validate_protocol(&protocol)?;
    Ok((protocol, i))
}
fn validate_protocol(p: &Protocol) -> R<()> {
    let diagnostic = |code: &'static str, message: &'static str| Diagnostic::new(p.pos, code, message);
    if !p.states.contains(&p.initial) || p.terminal.is_empty() ||
        p.terminal.iter().any(|s| !p.states.contains(s)) {
        return Err(diagnostic("E_PROTOCOL_STATE", "initial or terminal state is undeclared"));
    }
    if p.terminal.contains(&p.initial) {
        return Err(diagnostic("E_PROTOCOL_STATE", "initial state is terminal"));
    }
    if p.transitions.is_empty() {
        return Err(diagnostic("E_PROTOCOL_STATE", "protocol has no transitions"));
    }
    for transition in &p.transitions {
        if !p.states.contains(&transition.from) || !p.states.contains(&transition.to) {
            return Err(diagnostic("E_PROTOCOL_STATE", "transition has undeclared state"));
        }
        if p.terminal.contains(&transition.from) {
            return Err(diagnostic("E_PROTOCOL_STATE", "terminal state cannot send protocol messages"));
        }
    }
    let mut reachable = BTreeSet::from([p.initial.clone()]);
    let mut todo = VecDeque::new();
    todo.push_back(p.initial.clone());
    while let Some(state) = todo.pop_front() {
        for rule in p.transitions.iter().filter(|r| r.from == state) {
            if reachable.insert(rule.to.clone()) { todo.push_back(rule.to.clone()); }
        }
    }
    if reachable != p.states {
        return Err(diagnostic("E_PROTOCOL_STATE", "unreachable protocol state"));
    }
    let mut can_finish = p.terminal.clone();
    let mut changed = true;
    while changed {
        changed = false;
        for rule in &p.transitions {
            if can_finish.contains(&rule.to) && can_finish.insert(rule.from.clone()) {
                changed = true;
            }
        }
    }
    if can_finish != p.states {
        return Err(diagnostic("E_PROTOCOL_STATE", "state cannot reach terminal protocol state"));
    }
    Ok(())
}
impl Protocol {
    pub fn check_send(&self, possible: &[String], message: &str) -> R<SendProof> {
        if possible.is_empty() || possible.len() > MAX_STATES {
            return Err(Diagnostic::new(self.pos, "E_PROTOCOL_UNPROVEN_STATE",
                "message requires a bounded, statically known state set"));
        }
        let mut reply = None;
        let mut next = BTreeSet::new();
        for state in possible {
            if !self.states.contains(state) {
                return Err(Diagnostic::new(self.pos, "E_PROTOCOL_UNPROVEN_STATE",
                    format!("unknown state '{state}'")));
            }
            let rule = self.transitions.iter().find(|r| r.from == *state && r.message == message)
                .ok_or_else(|| Diagnostic::new(self.pos, "E_PROTOCOL_ILLEGAL_MESSAGE_STATE",
                    format!("{message} is not permitted in state {state}")))?;
            if reply.as_ref().is_some_and(|kind| kind != &rule.response) {
                return Err(Diagnostic::new(self.pos, "E_PROTOCOL_REPLY_MISMATCH",
                    "possible states disagree on response type or cardinality"));
            }
            reply = Some(rule.response.clone());
            next.insert(rule.to.clone());
        }
        Ok(SendProof {response:reply.expect("nonempty state proof"), next_states:next.into_iter().collect()})
    }
}
fn bindings(t: &[Token], index: &mut ProtocolIndex) -> R<()> {
    let mut i = 0;
    while i < t.len() {
        let decl = i + usize::from(word(t, i, "pub"));
        if !word(t, decl, "define") && !word(t, decl, "declare") {
            i += 1; continue;
        }
        let kind_at = decl + 1 + usize::from(word(t, decl + 1, "pub"));
        let kind = match at(t, kind_at) {
            Some(Tok::Ident(kind)) => kind.as_str(),
            _ => { i += 1; continue; }
        };
        if !matches!(kind, "actor" | "class" | "struct") {
            i += 1; continue;
        }
        let actor_name = match at(t, kind_at + 1) {
            Some(Tok::Ident(name)) => name.clone(),
            _ => { i += 1; continue; }
        };
        if !word(t, kind_at + 2, "implements") {
            i += 1; continue;
        }
        let mut j = kind_at + 3;
        loop {
            let contract = name(t, &mut j)?;
            if index.protocols.contains_key(&contract) {
                if kind != "actor" {
                    return Err(err(t, kind_at, "E_PROTOCOL_IMPLEMENTATION_MISMATCH",
                        "only the existing actor kind may implement a protocol"));
                }
                if index.actors.insert(actor_name.clone(), contract).is_some() {
                    return Err(err(t, kind_at, "E_PROTOCOL_DUPLICATE",
                        "actor has multiple protocol declarations"));
                }
            }
            if at(t, j) != Some(&Tok::Comma) { break; }
            j += 1;
        }
        i = j;
    }
    Ok(())
}
#[derive(Clone, Debug)]
struct Handle {
    protocol: String,
    states: Vec<String>,
}
#[derive(Clone, Debug)]
struct Pending {
    protocol: String,
    successor: Vec<String>,
    response: Response,
}
/// A bounded sequential proof checker for the proposal's call/await syntax.
/// It deliberately does not prove CFG joins, arbitrary methods or actual
/// mailbox processing; the compiler still rejects this feature for emission.
fn flow(t: &[Token], index: &mut ProtocolIndex) -> R<()> {
    let mut handles = BTreeMap::<String, Handle>::new();
    let mut consumed = BTreeSet::<String>::new();
    let mut pending = BTreeMap::<String, Pending>::new();
    // A settled ticket or consumed handle name cannot be recycled as a
    // syntactic workaround for affine/replay checks.
    let mut ever_named = BTreeSet::<String>::new();
    let mut completed = 0;
    let mut steps = 0;
    let mut i = 0;
    while i < t.len() {
        // val (reply, next_handle) = await pending?;
        if (word(t, i, "val") || word(t, i, "let")) &&
            at(t, i + 1) == Some(&Tok::LParen) &&
            at(t, i + 3) == Some(&Tok::Comma) &&
            at(t, i + 5) == Some(&Tok::RParen) &&
            at(t, i + 6) == Some(&Tok::Assign) && word(t, i + 7, "await")
        {
            let next_name = match at(t, i + 4) {
                Some(Tok::Ident(v)) => v.clone(),
                _ => return Err(err(t, i, "E_PROTOCOL_SYNTAX", "invalid successor binder"))
            };
            let ticket_name = match at(t, i + 8) {
                Some(Tok::Ident(v)) => v.clone(),
                _ => return Err(err(t, i, "E_PROTOCOL_SYNTAX", "expected pending call"))
            };
            if let Some(ticket) = pending.remove(&ticket_name) {
                if at(t, i + 9) != Some(&Tok::Question) {
                    return Err(err(t, i, "E_PROTOCOL_REPLY_REQUIRED",
                        "checked successful reply must precede next-state authority"));
                }
                if ever_named.contains(&next_name) || pending.contains_key(&next_name) {
                    return Err(err(t, i, "E_PROTOCOL_AFFINE_HANDLE", "successor handle name reused"));
                }
                // Awaiting success is not proof of execution here: this is a
                // *static* shape check; runtime must authenticate correlation.
                ever_named.insert(next_name.clone());
                handles.insert(next_name, Handle {
                    protocol:ticket.protocol, states:ticket.successor
                });
                steps += 1;
                i += 10; continue;
            }
        }
        // val h0 = await spawn Actor(args);
        if (word(t, i, "val") || word(t, i, "let")) &&
            matches!(at(t, i+1), Some(Tok::Ident(_))) &&
            at(t, i + 2) == Some(&Tok::Assign) &&
            word(t, i + 3, "await") && word(t, i + 4, "spawn")
        {
            let handle = match at(t, i+1) {Some(Tok::Ident(v)) => v.clone(), _ => unreachable!()};
            let actor = match at(t, i+5) {Some(Tok::Ident(v)) => v.clone(), _ => String::new()};
            if let Some(contract) = index.actors.get(&actor) {
                if word(t, i, "let") || ever_named.contains(&handle) ||
                    pending.contains_key(&handle) {
                    return Err(err(t, i, "E_PROTOCOL_AFFINE_HANDLE",
                        "protocol handle must use a unique immutable val binding"));
                }
                let initial = index.protocols[contract].initial.clone();
                ever_named.insert(handle.clone());
                handles.insert(handle, Handle {protocol:contract.clone(), states:vec![initial]});
                steps += 1;
                i += 6; continue;
            }
        }
        // val pending = handle.call(Message(args));
        if (word(t, i, "val") || word(t, i, "let")) &&
            matches!(at(t, i+1), Some(Tok::Ident(_))) &&
            at(t, i+2) == Some(&Tok::Assign) &&
            matches!(at(t, i+3), Some(Tok::Ident(_))) &&
            at(t, i+4) == Some(&Tok::Dot) && word(t, i+5, "call")
        {
            let ticket = match at(t, i+1) {Some(Tok::Ident(v)) => v.clone(), _ => unreachable!()};
            let handle_name = match at(t, i+3) {Some(Tok::Ident(v)) => v.clone(), _ => unreachable!()};
            if consumed.contains(&handle_name) {
                return Err(err(t, i+3, "E_PROTOCOL_AFFINE_HANDLE", "consumed protocol handle reused"));
            }
            if let Some(handle) = handles.remove(&handle_name) {
                if word(t, i, "let") || ever_named.contains(&ticket) ||
                    pending.contains_key(&ticket) {
                    return Err(err(t, i, "E_PROTOCOL_AFFINE_HANDLE",
                        "pending call must have a fresh immutable val binding"));
                }
                if at(t, i+6) != Some(&Tok::LParen) || !matches!(at(t, i+7), Some(Tok::Ident(_))) {
                    return Err(err(t, i, "E_PROTOCOL_SYNTAX", "call requires nominal message"));
                }
                let message = match at(t, i+7) {Some(Tok::Ident(v)) => v.clone(), _ => unreachable!()};
                let next = index.protocols[&handle.protocol].check_send(&handle.states, &message)?;
                if !matches!(&next.response, Response::Unary(_)) {
                    return Err(err(t, i+7, "E_PROTOCOL_REPLY_MISMATCH",
                        "stream transition requires an EOF-checked stream lease, not call()"));
                }
                if at(t, i+8) != Some(&Tok::LParen) {
                    return Err(err(t, i+8, "E_PROTOCOL_SYNTAX", "message must be constructed"));
                }
                let close_inner = group_end(t, i+8)?;
                if at(t, close_inner+1) != Some(&Tok::RParen) {
                    return Err(err(t, close_inner+1, "E_PROTOCOL_SYNTAX",
                        "expected end of state-changing call"));
                }
                consumed.insert(handle_name);
                ever_named.insert(ticket.clone());
                pending.insert(ticket, Pending {
                    protocol:handle.protocol, successor:next.next_states, response:next.response
                });
                steps += 1;
                i = close_inner + 2; continue;
            }
        }
        // terminal_handle.complete_protocol();
        if let Some(Tok::Ident(h)) = at(t, i) {
            if at(t, i+1) == Some(&Tok::Dot) && word(t, i+2, "complete_protocol") {
                if consumed.contains(h) {
                    return Err(err(t, i, "E_PROTOCOL_AFFINE_HANDLE", "consumed handle reused"));
                }
                if let Some(handle) = handles.remove(h) {
                    if handle.states.iter().any(|s| !index.protocols[&handle.protocol].terminal.contains(s)) {
                        return Err(err(t, i, "E_PROTOCOL_TERMINAL_INCOMPLETE",
                            "protocol completion requires a terminal state"));
                    }
                    if at(t, i+3) != Some(&Tok::LParen) || at(t, i+4) != Some(&Tok::RParen) {
                        return Err(err(t, i, "E_PROTOCOL_SYNTAX", "complete_protocol takes no arguments"));
                    }
                    consumed.insert(h.clone());
                    completed += 1;
                    steps += 1;
                    i += 5; continue;
                }
            }
            if consumed.contains(h) && at(t, i+1) == Some(&Tok::Dot) {
                return Err(err(t, i, "E_PROTOCOL_AFFINE_HANDLE", "consumed protocol handle reused"));
            }
            if handles.contains_key(h) {
                return Err(err(t, i, "E_PROTOCOL_UNPROVEN_STATE",
                    "protocol handle used outside a checked affine call or completion"));
            }
            if pending.contains_key(h) {
                return Err(err(t, i, "E_PROTOCOL_REPLY_REQUIRED",
                    "pending protocol call used without authenticated await/result binding"));
            }
        }
        // An affine authority cannot be duplicated via a plain binding,
        // copy!, copy!!, share! or an unchecked take!/cast.
        if (word(t, i, "val") || word(t, i, "let")) &&
            at(t, i+2) == Some(&Tok::Assign) {
            if let Some(Tok::Ident(v)) = at(t, i+3) {
                if handles.contains_key(v) || consumed.contains(v) {
                    return Err(err(t, i+3, "E_PROTOCOL_AFFINE_HANDLE",
                        "protocol state authority cannot be aliased"));
                }
            }
        }
        if ["copy", "share", "take", "borrow", "ref"].iter().any(|v| word(t, i, v)) &&
            at(t, i+1) == Some(&Tok::Bang) {
            if let Some(Tok::Ident(v)) = at(t, i+2) {
                if handles.contains_key(v) || consumed.contains(v) {
                    return Err(err(t, i+2, "E_PROTOCOL_AFFINE_HANDLE",
                        "protocol handle cannot be copied, shared or forged"));
                }
            }
        }
        i += 1;
        if steps > MAX_STEPS {
            return Err(err(t, i.saturating_sub(1), "E_PROTOCOL_BUDGET", "too many protocol proof steps"));
        }
    }
    if !pending.is_empty() {
        return Err(Diagnostic::new(t.last().map_or(Pos::default(), |x| x.pos),
            "E_PROTOCOL_REPLY_REQUIRED", "unresolved protocol reply obligation"));
    }
    if !handles.is_empty() {
        return Err(Diagnostic::new(t.last().map_or(Pos::default(), |x| x.pos),
            "E_PROTOCOL_TERMINAL_INCOMPLETE", "live protocol handles require completion or explicit transfer"));
    }
    index.completed_handles = completed;
    Ok(())
}
/// Parse actual AttoLang tokens; validate state machines and the restricted
/// sequential call syntax. No runtime code is permitted through this gate.
pub fn inspect_tokens(t: &[Token]) -> R<Option<ProtocolIndex>> {
    let mut index = ProtocolIndex::default();
    let mut i = 0;
    while i < t.len() {
        let protocol_header = (word(t, i, "define") &&
            (word(t, i+1, "protocol") ||
             (word(t, i+1, "pub") && word(t, i+2, "protocol")))) ||
            (word(t, i, "pub") && word(t, i+1, "define") &&
                word(t, i+2, "protocol"));
        if protocol_header {
            let (protocol, next) = parse_protocol(t, i)?;
            if index.protocols.insert(protocol.name.clone(), protocol).is_some() {
                return Err(err(t, i, "E_PROTOCOL_DUPLICATE", "duplicate protocol declaration"));
            }
            i = next;
        } else { i += 1; }
    }
    if index.protocols.is_empty() { return Ok(None); }
    bindings(t, &mut index)?;
    flow(t, &mut index)?;
    Ok(Some(index))
}
pub fn inspect_source(source: &str) -> R<Option<ProtocolIndex>> {
    inspect_tokens(&crate::lexer::lex(source)?)
}

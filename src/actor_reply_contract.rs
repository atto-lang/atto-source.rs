//! Typed, nominal actor application-reply contract preflight.
//! This is a bounded compiler-side IR, NOT a guest reply ABI.
//! Nominal mode comes only from explicitly implemented/inherited marker
//! contracts, never a method return, pipe emit or a string payload tag.
use std::collections::{BTreeMap, BTreeSet};
use crate::actor_reply_syntax::UnarySignature;
use crate::ast::{ActorMessageContract, ActorResponseMode};
use crate::diag::{Diagnostic, Pos};
use crate::lexer::{Tok, Token};

type R<T> = Result<T, Diagnostic>;
const MAX_NOMINALS: usize = 128;
const MAX_BASES: usize = 16;
const MAX_DEPTH: usize = 16;
const MAX_TYPE_LEN: usize = 512;

#[derive(Clone, Debug)]
struct Nominal {
    parents: Vec<String>,
    mode: Option<ActorResponseMode>,
    pos: Pos,
}
fn is(t: &[Token], i: usize, s: &str) -> bool {
    matches!(t.get(i).map(|x| &x.tok), Some(Tok::Ident(w)) if w == s)
}
fn is_tok(t: &[Token], i: usize, tok: &Tok) -> bool {
    t.get(i).is_some_and(|x| &x.tok == tok)
}
fn pos(t: &[Token], i: usize) -> Pos {
    t.get(i).map(|x| x.pos).unwrap_or_default()
}
fn diagnostic(t: &[Token], i: usize, code: &'static str, msg: impl Into<String>) -> Diagnostic {
    Diagnostic::new(pos(t, i), code, msg)
}
fn parse_type(t: &[Token], i: &mut usize, depth: usize) -> R<String> {
    if depth > MAX_DEPTH {
        return Err(diagnostic(t, *i, "E_NESTING", "nominal reply type nesting exceeded"));
    }
    let root = match t.get(*i).map(|x| &x.tok) {
        Some(Tok::Ident(s)) => s.clone(),
        _ => return Err(diagnostic(t, *i, "E_REPLY_TYPE", "expected a nominal message/reply type")),
    };
    *i += 1;
    let mut repr = root;
    if is_tok(t, *i, &Tok::Lt) {
        *i += 1;
        repr.push('<');
        let mut count = 0;
        loop {
            if count >= MAX_BASES {
                return Err(diagnostic(t, *i, "E_SOURCE_LIMIT", "too many nominal type arguments"));
            }
            if count > 0 { repr.push(','); }
            repr.push_str(&parse_type(t, i, depth + 1)?);
            count += 1;
            if is_tok(t, *i, &Tok::Comma) { *i += 1; continue; }
            if !is_tok(t, *i, &Tok::Gt) {
                return Err(diagnostic(t, *i, "E_REPLY_TYPE", "expected '>' after nominal reply type"));
            }
            *i += 1;
            repr.push('>');
            break;
        }
    }
    if repr.len() > MAX_TYPE_LEN {
        return Err(diagnostic(t, *i, "E_SOURCE_LIMIT", "nominal reply type too long"));
    }
    Ok(repr)
}
fn mode(ty: &str, p: Pos) -> R<Option<ActorResponseMode>> {
    if ty == "OneWayMessage" { return Ok(Some(ActorResponseMode::OneWay)); }
    for (name, stream) in [("UnaryRequest", false), ("StreamRequest", true)] {
        if ty == name {
            return Err(Diagnostic::new(p, "E_REPLY_TYPE", format!("{name} requires one type argument")));
        }
        let prefix = format!("{name}<");
        if let Some(item) = ty.strip_prefix(&prefix).and_then(|s| s.strip_suffix('>')) {
            if item.is_empty() || matches!(item, "void" | "any" | "Any") {
                return Err(Diagnostic::new(p, "E_REPLY_TYPE",
                    "reply type cannot be void or erased Any; use a concrete unit type"));
            }
            return Ok(Some(if stream {
                ActorResponseMode::Stream(item.to_owned())
            } else {
                ActorResponseMode::Unary(item.to_owned())
            }));
        }
    }
    Ok(None)
}
fn nominal_headers(t: &[Token]) -> R<BTreeMap<String, Nominal>> {
    let mut decls = BTreeMap::new();
    let mut i = 0;
    let mut braces = 0usize;
    while i < t.len() {
        if is_tok(t, i, &Tok::LBrace) { braces += 1; i += 1; continue; }
        if is_tok(t, i, &Tok::RBrace) { braces = braces.saturating_sub(1); i += 1; continue; }
        if braces > 0 || !is(t, i, "define") { i += 1; continue; }
        let mut j = i + 1;
        if is(t, j, "pub") || is(t, j, "private") { j += 1; }
        if !(is(t, j, "class") || is(t, j, "interface") || is(t, j, "trait")) {
            i += 1; continue;
        }
        j += 1;
        let name = parse_type(t, &mut j, 0)?;
        if name.contains('<') {
            // Generic nominal class/type definitions need the full type graph.
            i += 1; continue;
        }
        let mut parents = Vec::new();
        let mut direct = None;
        while is(t, j, "extends") || is(t, j, "implements") {
            j += 1;
            loop {
                if parents.len() >= MAX_BASES {
                    return Err(diagnostic(t, j, "E_SOURCE_LIMIT", "too many nominal message parents"));
                }
                let p = pos(t, j);
                let parent = parse_type(t, &mut j, 0)?;
                if let Some(found) = mode(&parent, p)? {
                    if direct.is_some() {
                        return Err(Diagnostic::new(p, "E_REPLY_MODE",
                            "message cannot implement multiple reply modes"));
                    }
                    direct = Some(found);
                } else {
                    if parents.contains(&parent) {
                        return Err(Diagnostic::new(p, "E_DUPLICATE", "duplicate nominal parent"));
                    }
                    parents.push(parent);
                }
                if !is_tok(t, j, &Tok::Comma) { break; }
                j += 1;
            }
        }
        if !is(t, j, "as") {
            // A non-standard class declaration is handled by its normal
            // compiler path; never guess an unparsed protocol.
            i += 1; continue;
        }
        if decls.len() >= MAX_NOMINALS {
            return Err(diagnostic(t, i, "E_SOURCE_LIMIT", "too many message type declarations"));
        }
        if decls.insert(name.clone(), Nominal { parents, mode: direct, pos: pos(t, i) }).is_some() {
            return Err(diagnostic(t, i, "E_DUPLICATE", format!("duplicate nominal message {name}")));
        }
        i = j + 1;
        // Skip the exact class body. Nested braces are ignored while looking
        // for its top-level end delimiter. No identifier in a string/comment
        // is visible here because the lexer has already classified tokens.
        let mut depth = 0usize;
        loop {
            if i >= t.len() || is_tok(t, i, &Tok::Eof) {
                return Err(diagnostic(t, j, "E_EOF", "unterminated nominal message declaration"));
            }
            if is_tok(t, i, &Tok::LBrace) { depth += 1; }
            if is_tok(t, i, &Tok::RBrace) { depth = depth.saturating_sub(1); }
            if depth == 0 && is(t, i, "end") { i += 1; break; }
            i += 1;
        }
    }
    Ok(decls)
}
fn resolve(name: &str, all: &BTreeMap<String, Nominal>,
    stack: &mut BTreeSet<String>) -> R<Option<ActorResponseMode>> {
    let Some(node) = all.get(name) else { return Ok(None); };
    if !stack.insert(name.to_owned()) {
        return Err(Diagnostic::new(node.pos, "E_REPLY_MODE",
            "cyclic nominal message inheritance cannot establish reply cardinality"));
    }
    let mut result = node.mode.clone();
    for parent in &node.parents {
        if let Some(inherited) = resolve(parent, all, stack)? {
            if result.is_some() {
                return Err(Diagnostic::new(node.pos, "E_REPLY_MODE",
                    "overlapping nominal reply modes in the message inheritance graph"));
            }
            result = Some(inherited);
        }
    }
    stack.remove(name);
    Ok(result)
}
/// Typecheck unary handlers against the actual nominal marker and its
/// inherited payload type, producing compiler-visible typed contract IR.
///
/// A legacy unary header with no visible request declaration stays source-
/// only and must still be rejected by the executable compiler. A class that
/// is visible but has no explicit mode is a hard type error.
pub fn check_unary(t: &[Token], signatures: &[UnarySignature])
    -> R<Vec<ActorMessageContract>> {
    let graph = nominal_headers(t)?;
    let mut out = Vec::new();
    for sig in signatures {
        let resolved = resolve(&sig.request, &graph, &mut BTreeSet::new())?;
        match resolved {
            Some(ActorResponseMode::Unary(response)) if response == sig.response => {
                out.push(ActorMessageContract {
                    actor: sig.actor.clone(),
                    message: sig.request.clone(),
                    response: ActorResponseMode::Unary(response),
                    pos: sig.pos,
                });
            }
            Some(ActorResponseMode::Unary(response)) => {
                return Err(Diagnostic::new(sig.pos, "E_REPLY_TYPE",
                    format!("{}: UnaryRequest<{response}> does not match handler response {}",
                        sig.request, sig.response)));
            }
            Some(ActorResponseMode::Stream(_)) | Some(ActorResponseMode::OneWay) => {
                return Err(Diagnostic::new(sig.pos, "E_REPLY_MODE",
                    format!("unary actor {} cannot receive non-unary message {}", sig.actor, sig.request)));
            }
            None if graph.contains_key(&sig.request) => {
                return Err(Diagnostic::new(sig.pos, "E_REPLY_MODE",
                    format!("{} has no nominal UnaryRequest<R> contract", sig.request)));
            }
            None => {} // unresolved external imports: fail closed at compilation gate
        }
    }
    Ok(out)
}
/// Resolve an application's declared message mode without granting access
/// to the runtime reply sidecar or supervisor CONTROL capability.
pub fn resolve_message_mode(t: &[Token], message: &str) -> R<Option<ActorResponseMode>> {
    resolve(message, &nominal_headers(t)?, &mut BTreeSet::new())
}

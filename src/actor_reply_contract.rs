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


/// Preflight ordinary actor mail reply sites against the message's NOMINAL
/// response mode. This is typed source IR only: arbitrary expressions and
/// control-flow linearity still require the full production checker.
pub fn check_explicit_replies(t: &[Token]) -> R<Vec<ActorMessageContract>> {
    let graph = nominal_headers(t)?;
    let mut out = Vec::new();
    let mut i = 0;
    while i + 8 < t.len() {
        if !(is(t, i, "receive") && is_tok(t, i + 1, &Tok::LParen)
            && is(t, i + 2, "ActorMail") && is_tok(t, i + 3, &Tok::Lt)
            && is_tok(t, i + 5, &Tok::Gt) && is_tok(t, i + 7, &Tok::RParen)) {
            i += 1; continue;
        }
        let (message, mail) = match (&t[i + 4].tok, &t[i + 6].tok) {
            (Tok::Ident(msg), Tok::Ident(var)) => (msg.as_str(), var.as_str()),
            _ => { i += 1; continue; },
        };
        let mut body_start = i + 8;
        if is_tok(t, body_start, &Tok::Colon) || is_tok(t, body_start, &Tok::Arrow) {
            body_start += 1;
            if !is(t, body_start, "void") {
                // Nonvoid return is handled by the unary actor validator,
                // not by ordinary reply sugar.
                i += 1; continue;
            }
            body_start += 1;
        }
        if !is_tok(t, body_start, &Tok::LBrace) { i += 1; continue; }
        let mut end = body_start + 1;
        let mut depth = 1usize;
        while end < t.len() && depth > 0 {
            match t[end].tok {
                Tok::LBrace => depth += 1,
                Tok::RBrace => depth -= 1,
                Tok::Eof => return Err(diagnostic(t, i, "E_EOF", "unclosed ActorMail receive body")),
                _ => {},
            }
            end += 1;
        }
        if depth != 0 { return Err(diagnostic(t, i, "E_EOF", "unclosed ActorMail receive body")); }
        let mut saw_reply = false;
        let mut first_kind = "";
        let mut site_count = 0usize;
        for j in body_start + 1..end.saturating_sub(2) {
            if !is(t, j, mail) || !is_tok(t, j + 1, &Tok::Dot) { continue; }
            let kind = if is(t, j + 2, "respond_with") { "unary" }
                else if is(t, j + 2, "reply_stream") { "stream" }
                else if is(t, j + 2, "send_back") { "alias" }
                else { continue };
            saw_reply = true;
            site_count += 1;
            if first_kind.is_empty() { first_kind = kind; }
            if first_kind != kind {
                return Err(diagnostic(t, j, "E_REPLY_LINEAR",
                    "mixed reply primitives cannot claim multiple reply permissions"));
            }
            if !is_tok(t, j + 3, &Tok::LParen) {
                return Err(diagnostic(t, j + 3, "E_REPLY_SIGNATURE",
                    "reply method requires a parenthesized argument list"));
            }
            if kind == "unary" {
                // Check literal/concrete constructor types, but do not guess
                // the inferred type of a variable or arbitrary expression.
                let candidate = if is(t, j + 4, "new") {
                    match t.get(j + 5).map(|x| &x.tok) {
                        Some(Tok::Ident(n)) => Some(n.clone()),
                        _ => None,
                    }
                } else {
                    match t.get(j + 4).map(|x| &x.tok) {
                        Some(Tok::Int(_)) => Some("int".to_owned()),
                        Some(Tok::Str(_)) => Some("string".to_owned()),
                        Some(Tok::Ident(n)) if n == "true" || n == "false" =>
                            Some("bool".to_owned()),
                        _ => None,
                    }
                };
                if let Some(ActorResponseMode::Unary(expected)) =
                    resolve(message, &graph, &mut BTreeSet::new())? {
                    if let Some(actual) = candidate {
                        if expected != actual {
                            return Err(diagnostic(t, j, "E_REPLY_TYPE",
                                format!("respond_with expects {expected}, found {actual}")));
                        }
                    } else if is_tok(t, j + 4, &Tok::RParen) && expected != "Unit" {
                        return Err(diagnostic(t, j, "E_REPLY_TYPE",
                            "respond_with() is only valid for a unit-valued unary request"));
                    }
                }
            }
        }
        if saw_reply {
            let response = resolve(message, &graph, &mut BTreeSet::new())?;
            match (&response, first_kind) {
                (Some(ActorResponseMode::Unary(_)), "unary") => {},
                (Some(ActorResponseMode::Stream(_)), "stream") => {},
                (None, _) if !graph.contains_key(message) => {
                    i = end; continue; // imported/unknown: no proven authority
                }
                _ => return Err(diagnostic(t, i, "E_REPLY_MODE",
                    format!("ActorMail<{message}> has no matching {first_kind} reply permission"))),
            }
            // A second potential reply on a path requires ownership/CFG
            // analysis. Reject conservatively without alleging that different
            // match arms necessarily execute together.
            if site_count > 1 {
                return Err(diagnostic(t, i, "E_UNSUPPORTED",
                    "multiple reply sites require a path-sensitive linearity checker"));
            }
            let actor = (0..i).rev().find_map(|k| {
                if is(t, k, "define") && is(t, k + 1, "actor") {
                    if let Some(Token { tok: Tok::Ident(name), .. }) = t.get(k + 2) {
                        return Some(name.clone());
                    }
                }
                None
            }).unwrap_or_else(|| "<unlinked actor>".to_owned());
            out.push(ActorMessageContract {
                actor, message: message.to_owned(), response: response.unwrap(), pos: pos(t, i),
            });
        }
        i = end;
    }
    Ok(out)
}

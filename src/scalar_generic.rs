//! Bounded scalar generic specialization before the production Rust parser.
//! This does not implement general generics. Only braced, source-local
//! named callables with one unconstrained type parameter are eligible.
//! Every instantiated body is checked by the normal parser and Wasm backend.

use std::collections::BTreeMap;
use crate::diag::{Diagnostic, Pos};
use crate::lexer::{Tok, Token, MAX_TOKENS};

const MAX_GENERIC_FUNCTIONS: usize = 16;
const MAX_INSTANCES: usize = 64;

#[derive(Clone)]
struct Template {
    name: String,
    parameter: String,
    start: usize,
    end: usize,
    name_offset: usize,
    source: Vec<Token>,
    pos: Pos,
}

fn word(t: &Token) -> Option<&str> {
    if let Tok::Ident(s) = &t.tok { Some(s) } else { None }
}

fn error(pos: Pos, code: &'static str, why: &str) -> Diagnostic {
    Diagnostic::new(pos, code, why)
}

fn pair(tokens: &[Token], at: usize, open: Tok, close: Tok) -> Option<usize> {
    if tokens.get(at)?.tok != open { return None; }
    let mut depth = 0;
    for (i, token) in tokens.iter().enumerate().skip(at) {
        if token.tok == open { depth += 1; }
        if token.tok == close {
            depth -= 1;
            if depth == 0 { return Some(i); }
        }
        if token.tok == Tok::Eof { return None; }
    }
    None
}

fn scalar(t: &Token) -> Option<&'static str> {
    match word(t)? {
        "int" => Some("int"),
        "float" | "f64" => Some("float"),
        "bool" | "boolean" => Some("bool"),
        "string" | "String" => Some("string"),
        _ => None,
    }
}

fn template_at(tokens: &[Token], i: usize) -> Result<Option<Template>, Diagnostic> {
    if word(&tokens[i]) != Some("fnc") { return Ok(None); }
    let Some(name) = tokens.get(i + 1).and_then(word) else { return Ok(None) };
    if tokens.get(i + 2).map(|t| &t.tok) != Some(&Tok::Lt) {
        return Ok(None);
    }
    let parameter = tokens.get(i + 3).and_then(word)
        .ok_or_else(|| error(tokens[i].pos, "E_UNSUPPORTED", "generic type parameter required"))?;
    if tokens.get(i + 4).map(|t| &t.tok) != Some(&Tok::Gt) ||
        tokens.get(i + 5).map(|t| &t.tok) != Some(&Tok::LParen) {
        return Err(error(tokens[i].pos, "E_UNSUPPORTED",
            "browser generics require one unconstrained type parameter"));
    }
    if scalar(&tokens[i + 3]).is_some() || name == parameter ||
        crate::parser::is_reserved(parameter) {
        return Err(error(tokens[i + 3].pos, "E_TYPE",
            "generic type parameter conflicts with a reserved type or function name"));
    }
    let params_end = pair(tokens, i + 5, Tok::LParen, Tok::RParen)
        .ok_or_else(|| error(tokens[i].pos, "E_SYNTAX", "unclosed generic parameters"))?;
    let mut body = params_end + 1;
    while body < tokens.len() && tokens[body].tok != Tok::LBrace {
        if matches!(tokens[body].tok, Tok::Semi | Tok::ColonColon | Tok::Eof) {
            return Err(error(tokens[i].pos, "E_UNSUPPORTED", "generic function needs a braced body"));
        }
        body += 1;
    }
    if body == tokens.len() { return Err(error(tokens[i].pos, "E_SYNTAX", "generic function body missing")); }
    let end = pair(tokens, body, Tok::LBrace, Tok::RBrace)
        .ok_or_else(|| error(tokens[i].pos, "E_SYNTAX", "unclosed generic body"))? + 1;
    let mut start = i;
    if i > 0 && word(&tokens[i - 1]) == Some("pure") { start -= 1; }
    if start > 0 && word(&tokens[start - 1]) == Some("pub") { start -= 1; }
    Ok(Some(Template { name: name.into(), parameter: parameter.into(),
        start, end, name_offset: i + 1 - start, source: tokens[start..end].to_vec(),
        pos: tokens[i].pos }))
}

fn context(tokens: &[Token], at: usize, end: usize) -> Option<&'static str> {
    let next = end + 1;
    if tokens.get(next).map(|t| &t.tok) == Some(&Tok::Colon) ||
        tokens.get(next).and_then(word) == Some("as") {
        // A plain ternary colon is not a type-ascription operator.
        let mut nesting = 0usize;
        for t in tokens[..at].iter().rev() {
            match t.tok {
                Tok::RParen => nesting += 1,
                Tok::LParen if nesting > 0 => nesting -= 1,
                Tok::Semi | Tok::LBrace if nesting == 0 => break,
                Tok::Question if nesting == 0 => return None,
                _ => {}
            }
        }
        return tokens.get(next + 1).and_then(scalar);
    }
    // Direct typed binding: val/let/const result: T = foo(args).
    let before = &tokens[..at];
    let begin = before.iter().rposition(|t| matches!(t.tok, Tok::Semi | Tok::LBrace))
        .map_or(0, |n| n + 1);
    let head = &before[begin..];
    if head.len() == 5 &&
        matches!(&head[0].tok, Tok::Ident(w) if matches!(w.as_str(), "val" | "let" | "const")) &&
        matches!(&head[1].tok, Tok::Ident(_)) &&
        head[2].tok == Tok::Colon && head[4].tok == Tok::Assign {
        return scalar(&head[3]);
    }
    None
}

fn specialize(template: &Template, to: &str, private_name: &str) -> Vec<Token> {
    let mut result = template.source.clone();
    result[template.name_offset].tok = Tok::Ident(private_name.into());
    result.drain(template.name_offset + 1..template.name_offset + 4);
    for token in &mut result {
        if word(token) == Some(template.parameter.as_str()) {
            token.tok = Tok::Ident(to.into());
        }
    }
    if word(&result[0]) == Some("pub") { result.remove(0); }
    result
}

/// Rewrites direct generic invocations into source-local monomorphic instances.
/// Underconstrained calls and unverified uninstantiated bodies fail closed.
pub fn lower(tokens: Vec<Token>) -> Result<Vec<Token>, Diagnostic> {
    let mut templates = Vec::<Template>::new();
    let mut i = 0;
    let mut braces = 0usize;
    let mut nominal_blocks = 0usize;
    while i < tokens.len() {
        if word(&tokens[i]) == Some("define") && tokens.get(i + 1).and_then(word)
            .is_some_and(|kind| matches!(kind, "class" | "actor" | "enum" | "module")) {
            nominal_blocks += 1;
        }
        if word(&tokens[i]) == Some("end") && braces == 0 && nominal_blocks > 0 {
            nominal_blocks -= 1;
        }
        if tokens[i].tok == Tok::LBrace { braces += 1; }
        if tokens[i].tok == Tok::RBrace { braces = braces.saturating_sub(1); }
        if let Some(template) = template_at(&tokens, i)? {
            if braces > 0 || nominal_blocks > 0 {
                return Err(error(template.pos, "E_UNSUPPORTED",
                    "generic methods or nested generic functions are not supported by the browser compiler"));
            }
            if templates.len() == MAX_GENERIC_FUNCTIONS {
                return Err(error(template.pos, "E_SOURCE_LIMIT", "too many generic function templates"));
            }
            if templates.iter().any(|t| t.name == template.name) {
                return Err(error(template.pos, "E_DUPLICATE", "duplicate generic function template"));
            }
            i = template.end;
            templates.push(template);
        } else { i += 1; }
    }
    if templates.is_empty() { return Ok(tokens); }
    let mut base = Vec::new();
    let mut begin = 0;
    for template in &templates {
        base.extend_from_slice(&tokens[begin..template.start]);
        begin = template.end;
    }
    base.extend_from_slice(&tokens[begin..]);
    // There is no safe overloading or shadowed callable resolution in the
    // bounded specializer. A same-named concrete callable or binding must
    // not have its references rewritten into a different generic function.
    for template in &templates {
        if base.windows(2).any(|w| {
            (word(&w[0]) == Some("fnc") || word(&w[0]) == Some("routine")) &&
                word(&w[1]) == Some(template.name.as_str())
        }) || base.windows(2).any(|w| {
            matches!(word(&w[0]), Some("val" | "let" | "const")) &&
                word(&w[1]) == Some(template.name.as_str())
        }) {
            return Err(error(template.pos, "E_DUPLICATE",
                "generic function name conflicts with a callable or local binding"));
        }
    }
    let eof = base.pop().ok_or_else(|| error(Pos::default(), "E_SYNTAX", "empty source"))?;
    if eof.tok != Tok::Eof {
        return Err(error(eof.pos, "E_SYNTAX", "missing end-of-source"));
    }
    let mut out = Vec::new();
    let mut requested: BTreeMap<(usize, String), String> = BTreeMap::new();
    let mut at = 0;
    while at < base.len() {
        let tid = word(&base[at]).and_then(|name|
            templates.iter().position(|template| template.name == name));
        let Some(tid) = tid else {
            out.push(base[at].clone());
            at += 1;
            continue;
        };
        let mut args_at = at + 1;
        let explicit = if base.get(args_at).map(|t| &t.tok) == Some(&Tok::Lt) {
            let chosen = base.get(args_at + 1).and_then(scalar)
                .ok_or_else(|| error(base[at].pos, "E_UNSUPPORTED_TYPE",
                    "generic type argument must be an admitted scalar"))?;
            if base.get(args_at + 2).map(|t| &t.tok) != Some(&Tok::Gt) {
                return Err(error(base[at].pos, "E_UNSUPPORTED", "generic call needs one type argument"));
            }
            args_at += 3;
            Some(chosen)
        } else { None };
        if base.get(args_at).map(|t| &t.tok) != Some(&Tok::LParen) {
            return Err(error(base[at].pos, "E_INFER", "generic callable cannot escape as a value"));
        }
        let end = pair(&base, args_at, Tok::LParen, Tok::RParen)
            .ok_or_else(|| error(base[at].pos, "E_SYNTAX", "unclosed generic call"))?;
        let inferred = context(&base, at, end);
        if explicit.is_some() && inferred.is_some() && explicit != inferred {
            return Err(error(base[at].pos, "E_TYPE", "explicit type argument conflicts with expected result"));
        }
        let to = explicit.or(inferred).ok_or_else(|| error(base[at].pos, "E_INFER",
            "unknown generic result type: use <Type>, : Type, as Type or typed binding"))?;
        let key = (tid, to.to_string());
        if !requested.contains_key(&key) {
            if requested.len() == MAX_INSTANCES {
                return Err(error(base[at].pos, "E_SOURCE_LIMIT", "too many generic instances"));
            }
            let generated = format!("__atto_generic_{tid}_{to}");
            if tokens.iter().any(|t| word(t) == Some(generated.as_str())) {
                return Err(error(base[at].pos, "E_DUPLICATE", "generated generic name collides with source"));
            }
            requested.insert(key.clone(), generated);
        }
        let generated = requested.get(&key).expect("specialization inserted");
        out.push(Token { tok: Tok::Ident(generated.clone()), pos: base[at].pos });
        out.extend_from_slice(&base[args_at..=end]);
        at = end + 1;
    }
    if templates.iter().enumerate().any(|(tid, _)|
        !requested.keys().any(|key| key.0 == tid)) {
        return Err(error(templates[0].pos, "E_UNSUPPORTED",
            "uninstantiated generic definition cannot be type-checked in this browser profile"));
    }
    for ((tid, to), generated) in &requested {
        out.extend(specialize(&templates[*tid], to, generated));
    }
    out.push(eof);
    if out.len() > MAX_TOKENS {
        return Err(error(Pos::default(), "E_SOURCE_LIMIT", "expanded generics exceed token budget"));
    }
    Ok(out)
}

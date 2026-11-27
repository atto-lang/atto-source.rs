//! Bounded local Pair<int> scalar replacement, before the ordinary typed compiler.
//! Only one flat main function, immutable scalar payloads, literal keys,
//! typed aliases and read-only projections. No general Pair ABI or escape.
//! The source binding table retains the nominal Pair identity; only its
//! checked i64 payload reaches the existing scalar AST and Wasm emitter.
//! Unsupported syntax is rejected, not approximated or treated as a scalar.
use crate::diag::{Diagnostic, Pos};
use crate::lexer::{Tok, Token, MAX_TOKENS};
use crate::parser::is_reserved;
use std::collections::BTreeMap;

pub const PREFIX: &str = "__atto_pair_local_";

#[derive(Clone)]
struct Pair {
    key: String,
    payload: String,
    qualifier: String,
}
#[derive(Clone)]
enum Local { Scalar, Pair(Pair) }

fn ident(s: &str) -> Tok { Tok::Ident(s.to_owned()) }
fn tok(t: Tok, pos: Pos) -> Token { Token { tok: t, pos } }
fn err(pos: Pos, code: &'static str, message: &str) -> Diagnostic {
    Diagnostic::new(pos, code, message)
}
fn unsupported(pos: Pos) -> Diagnostic {
    err(pos, "E_PAIR_STORAGE_UNSUPPORTED", "bounded Pair<int> profile requires flat main, local scalar payloads and read-only projections")
}
fn is(t: Option<&Token>, want: &Tok) -> bool { t.is_some_and(|t| &t.tok == want) }
fn word(t: Option<&Token>, want: &str) -> bool {
    matches!(t.map(|t| &t.tok), Some(Tok::Ident(s)) if s == want)
}
fn name(t: &Token) -> Result<String, Diagnostic> {
    match &t.tok {
        Tok::Ident(s) if !is_reserved(s) && s != "Pair" && !s.starts_with(PREFIX) => Ok(s.clone()),
        _ => Err(unsupported(t.pos)),
    }
}
fn key(t: &Token) -> Result<String, Diagnostic> {
    match &t.tok {
        Tok::Str(s) => Ok(s.clone()),
        Tok::Ident(s) if !is_reserved(s) => Ok(s.clone()),
        _ => Err(err(t.pos, "E_PAIR_KEY_TYPE", "Pair key must be a literal identifier or string")),
    }
}
fn scalar_literal(ts: &[Token]) -> bool {
    matches!(ts, [Token { tok: Tok::Int(Some(_)), .. }])
        || matches!(ts, [Token { tok: Tok::Minus, .. }, Token { tok: Tok::Int(Some(_)), .. }])
}

// Explicitly whitelist the scope-free scalar expression subset. Reject
// every raw Pair use and every other projection, ownership or call shape.
// A projected key is a string literal; a projected value is the immutable
// scalar slot, so reading it never evaluates its original initializer again.
fn expression(ts: &[Token], locals: &BTreeMap<String, Local>, out: &mut Vec<Token>) -> Result<(), Diagnostic> {
    let mut i = 0;
    while i < ts.len() {
        let t = &ts[i];
        if let Tok::Ident(s) = &t.tok {
            match locals.get(s) {
                Some(Local::Pair(p)) => {
                    if !is(ts.get(i+1), &Tok::Dot) {
                        return Err(err(t.pos, "E_PAIR_ESCAPE_UNSUPPORTED", "a Pair cannot be implicitly extracted or escape this local profile"));
                    }
                    if word(ts.get(i+2), "key") {
                        out.push(tok(Tok::Str(p.key.clone()), t.pos));
                    } else if word(ts.get(i+2), "value") {
                        out.push(tok(ident(&p.payload), t.pos));
                    } else {
                        return Err(err(t.pos, "E_PAIR_PROJECTION_UNSUPPORTED", "only read-only key and value are admitted"));
                    }
                    i += 3;
                    continue;
                }
                Some(Local::Scalar) => {}
                None if matches!(s.as_str(), "true" | "false" | "if" | "then" | "else" | "fi" | "eq" | "neq") => {}
                None => return Err(err(t.pos, "E_UNBOUND", "unresolved scalar value operand in local Pair profile")),
            }
        } else if !matches!(&t.tok,
            Tok::Int(_) | Tok::Float(_) | Tok::Str(_) | Tok::LParen | Tok::RParen |
            Tok::Plus | Tok::Minus | Tok::Star | Tok::Slash | Tok::Bang |
            Tok::EqEq | Tok::NotEq | Tok::Lt | Tok::Le | Tok::Gt | Tok::Ge |
            Tok::Question | Tok::Colon) {
            return Err(unsupported(t.pos));
        }
        out.push(t.clone());
        i += 1;
    }
    Ok(())
}

fn factory(ts: &[Token]) -> bool {
    word(ts.first(), "Pair") && is(ts.get(1), &Tok::Dot) && word(ts.get(2), "of")
}
fn binding_factory(ts: &[Token]) -> bool {
    ts.windows(6).any(|w| matches!(&w[0].tok, Tok::Ident(s) if matches!(s.as_str(), "const"|"val"|"let"))
        && matches!(&w[1].tok, Tok::Ident(_)) && w[2].tok == Tok::Assign && factory(&w[3..]))
}

pub fn lower(ts: Vec<Token>) -> Result<Vec<Token>, Diagnostic> {
    // Compiler-owned names may not be minted by user source, even without Pair.
    for t in &ts {
        if matches!(&t.tok, Tok::Ident(s) if s.starts_with(PREFIX)) {
            return Err(err(t.pos, "E_RESERVED", "compiler-owned Pair scalar slot name"));
        }
    }
    if !ts.iter().any(|t| t.tok == Tok::ColonEqual) && !binding_factory(&ts) {
        return Ok(ts);
    }
    let header = [ident("pub"),ident("fnc"),ident("main"),Tok::LParen,Tok::RParen,Tok::Colon,ident("int"),Tok::LBrace];
    let pos = ts.first().map(|t| t.pos).unwrap_or_default();
    if ts.len() < header.len()+2 || !ts.iter().zip(&header).all(|(a,b)| &a.tok == b)
        || !is(ts.get(ts.len()-2), &Tok::RBrace) || !is(ts.last(), &Tok::Eof) {
        return Err(unsupported(pos));
    }
    let mut out = ts[..header.len()].to_vec();
    let mut locals = BTreeMap::<String, Local>::new();
    let mut ordinal = 0usize;
    let mut returned = false;
    let body = &ts[header.len()..ts.len()-2];
    let mut statements = body.split_inclusive(|t| t.tok == Tok::Semi);
    for statement in &mut statements {
        let pos = statement[0].pos;
        if returned || !is(statement.last(), &Tok::Semi) || statement.len() < 2 {
            return Err(unsupported(pos));
        }
        let s = &statement[..statement.len()-1];
        if word(s.first(), "return") {
            out.push(s[0].clone());
            expression(&s[1..], &locals, &mut out)?;
            returned = true;
        } else {
            let qualifier = match &s[0].tok {
                Tok::Ident(q) if matches!(q.as_str(), "const"|"val"|"let") => q.as_str(),
                _ => return Err(unsupported(pos)),
            };
            // Optional explicit primitive type or exact Pair<int> alias.
            let typed_pair = word(s.get(1), "Pair") && is(s.get(2), &Tok::Lt)
                && word(s.get(3), "int") && is(s.get(4), &Tok::Gt);
            let typed_scalar = matches!(s.get(1).map(|t| &t.tok),
                Some(Tok::Ident(t)) if matches!(t.as_str(), "int"|"bool"|"string"|"float"))
                && matches!(s.get(2).map(|t| &t.tok), Some(Tok::Ident(_)));
            let ni = if typed_pair {5} else if typed_scalar {2} else {1};
            let binder = s.get(ni).ok_or_else(|| unsupported(pos))?;
            let n = name(binder)?;
            let op = s.get(ni+1).ok_or_else(|| unsupported(pos))?;
            let rhs = &s[ni+2..];
            let mut pair_init: Option<(String, Vec<Token>, bool)> = None;
            if op.tok == Tok::ColonEqual && !typed_pair && !typed_scalar {
                pair_init = Some((n.clone(), rhs.to_vec(), false));
            } else if op.tok == Tok::Assign {
                if !typed_scalar && !typed_pair && is(rhs.get(1), &Tok::ColonEqual) {
                    pair_init = Some((key(&rhs[0])?, rhs[2..].to_vec(), false));
                } else if !typed_scalar && !typed_pair && factory(rhs) {
                    // Factory accepts a literal string key only in this profile.
                    if rhs.len()<7 || !is(rhs.get(3), &Tok::LParen) || !is(rhs.get(5), &Tok::Comma) || !is(rhs.last(), &Tok::RParen) {
                        return Err(unsupported(pos));
                    }
                    let Tok::Str(k) = &rhs[4].tok else { return Err(unsupported(rhs[4].pos)); };
                    pair_init = Some((k.clone(), rhs[6..rhs.len()-1].to_vec(), false));
                } else if typed_pair {
                    let [Token { tok: Tok::Ident(origin), .. }] = rhs else { return Err(unsupported(pos)); };
                    let Some(Local::Pair(p)) = locals.get(origin) else { return Err(unsupported(pos)); };
                    if qualifier == "const" && p.qualifier != "const" {
                        return Err(err(pos, "E_PAIR_CONSTEXPR", "const Pair alias requires a proven constant source"));
                    }
                    pair_init = Some((p.key.clone(), vec![tok(ident(&p.payload), pos)], true));
                }
            } else {
                return Err(unsupported(op.pos));
            }

            // Only repeated let Pair declarations are supported shadowing in
            // this initial profile. Never overwrite a previous immutable name.
            if let Some(prior) = locals.get(&n) {
                if !(qualifier == "let" && pair_init.is_some()
                    && matches!(prior, Local::Pair(p) if p.qualifier == "let")) {
                    return Err(err(binder.pos, "E_DUPLICATE", "duplicate local binding in Pair profile"));
                }
            }
            if let Some((k,payload,alias)) = pair_init {
                if payload.is_empty() { return Err(unsupported(pos)); }
                let mut checked_payload = Vec::new();
                if alias { checked_payload.extend(payload.iter().cloned()); }
                else { expression(&payload,&locals,&mut checked_payload)?; }
                if qualifier == "const" && !alias && !scalar_literal(&payload) {
                    return Err(err(pos,"E_PAIR_CONSTEXPR","const Pair payload requires a proven integer literal in this profile"));
                }
                let slot = format!("{PREFIX}{ordinal}");
                ordinal += 1;
                // No early source binding: rewrite payload with the previous
                // environment, then introduce the fresh Pair binding.
                out.extend([tok(ident("val"), pos),tok(ident("int"),pos),tok(ident(&slot),binder.pos),tok(Tok::Assign,op.pos)]);
                out.extend(checked_payload);
                locals.insert(n, Local::Pair(Pair { key:k, payload:slot, qualifier:qualifier.to_owned() }));
            } else {
                if typed_pair || op.tok != Tok::Assign { return Err(unsupported(pos)); }
                out.extend_from_slice(&s[..ni+2]);
                expression(rhs,&locals,&mut out)?;
                locals.insert(n,Local::Scalar);
            }
        }
        out.push(statement.last().unwrap().clone());
        if out.len() > MAX_TOKENS { return Err(unsupported(pos)); }
    }
    if !returned { return Err(unsupported(pos)); }
    out.extend_from_slice(&ts[ts.len()-2..]);
    if out.len() > MAX_TOKENS { return Err(unsupported(pos)); }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn source(body: &str) -> String { format!("pub fnc main(): int {{ {body} }}") }
    fn compile(body: &str) -> Result<Vec<u8>,Diagnostic> { crate::compile(&source(body)) }
    #[test]
    fn both_declarations_and_factory_emit_same_wasm() {
        let tail = "const Pair<int> p = x; return p.key == \"x\" ? p.value : 99;";
        let a = compile(&format!("const x := 5; {tail}")).unwrap();
        let b = compile(&format!("const x = x := 5; {tail}")).unwrap();
        let c = compile(&format!("const x = Pair.of(\"x\", 5); {tail}")).unwrap();
        assert_eq!(&a[..4], b"\0asm");
        assert_eq!(a,b); assert_eq!(b,c);
    }
    #[test]
    fn runtime_local_payload_and_fresh_shadow_compile() {
        assert!(compile("val n = 21; val x := n+n; return x.value;").is_ok());
        assert!(compile("let x := 5; let x := x.value + 1; return x.value;").is_ok());
    }
    #[test]
    fn invalid_cases_are_not_scalarized() {
        for body in [
            "const x := 5; return x;",
            "const x := 5; const x := 6; return x.value;",
            "const x := x; return x.value;",
            "const x = x := x; return x.value;",
            "val n = 5; const x := n; return x.value;",
            "val x := true; return x.value;",
            "let x := 5; x.key = \"y\"; return x.value;",
            "const x := y := 5; return x.value;",
            "val x := 5; const Pair<int> p = x; return p.value;",
        ] { assert!(compile(body).is_err(), "unsafe acceptance: {body}"); }
        assert_eq!(compile("const x := x; return x.value;").unwrap_err().code, "E_UNBOUND");
        assert_eq!(compile("const x = x := x; return x.value;").unwrap_err().code, "E_UNBOUND");
        assert!(compile("val __atto_pair_local_0 = 5; return 5;").is_err());
    }
}

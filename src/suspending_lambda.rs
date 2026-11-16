//! Bounded direct-call suspension lambda lowering; no runtime closure values.
//!
//! Accepts a file-local `val named = suspends |int x| -> scalar_expr;`
//! binding in a braced callable body. The binding is erased only after proving
//! that all uses in that same body are direct calls. The typed helper is a
//! real `suspends fnc`, so normal effect checks and JSPI cooperative handoff
//! still apply. Captures fail closed through the regular name/type checker.
use crate::diag::{Diagnostic, Pos};
use crate::lexer::{Tok, Token};
type R<T> = Result<T, Diagnostic>;

fn word(t: &[Token], i: usize, w: &str) -> bool {
    matches!(t.get(i).map(|v| &v.tok), Some(Tok::Ident(s)) if s == w)
}
fn fail(p: Pos, code: &'static str, m: impl Into<String>) -> Diagnostic {
    Diagnostic::new(p, code, m)
}
fn tok(t: Tok, p: Pos) -> Token { Token { tok: t, pos: p } }

/// Intentionally one source-local, single-int-argument direct lambda profile;
/// this must not silently grant arbitrary callable transport/capture semantics.
pub fn lower(t: Vec<Token>) -> R<Vec<Token>> {
    let maybe = t.iter().any(|x| matches!(&x.tok, Tok::Ident(w)
        if w == "suspends" || w == "susp"));
    if !maybe { return Ok(t); }
    // All compiler-owned generated symbols are unlexable by source, but
    // reject API-level token injection/forged names too.
    if let Some(x) = t.iter().find(|x| matches!(&x.tok, Tok::Ident(n)
        if n.starts_with("$atto_susp_lambda_"))) {
        return Err(fail(x.pos, "E_RESERVED", "synthetic suspending lambda names cannot be supplied by source"));
    }

    let mut found: Option<(usize, usize, usize, String, String, Vec<Token>, Pos)> = None;
    let mut depth = 0usize;
    let mut scope_open = 0usize;
    let mut i = 0usize;
    while i < t.len() {
        if t[i].tok == Tok::LBrace {
            if depth == 0 { scope_open = i; }
            depth += 1;
        } else if t[i].tok == Tok::RBrace {
            depth = depth.saturating_sub(1);
        }
        if !word(&t, i, "val") ||
            !matches!(t.get(i+1).map(|x| &x.tok), Some(Tok::Ident(_))) ||
            t.get(i+2).map(|x| &x.tok) != Some(&Tok::Assign) ||
            !(word(&t, i+3, "suspends") || word(&t, i+3, "susp")) {
            i += 1;
            continue;
        }
        if found.is_some() {
            return Err(fail(t[i].pos, "E_UNSUPPORTED",
                "only one nonescaping suspending lambda is currently supported per source"));
        }
        if depth != 1 {
            return Err(fail(t[i].pos, "E_UNSUPPORTED",
                "suspending lambda direct binding requires a braced callable body"));
        }
        let Tok::Ident(binding) = &t[i+1].tok else { unreachable!() };
        if binding == "self" {
            return Err(fail(t[i+1].pos, "E_RESERVED", "cannot bind a lambda to self"));
        }
        if t.get(i+4).map(|x| &x.tok) != Some(&Tok::Pipe) ||
            !word(&t, i+5, "int") ||
            !matches!(t.get(i+6).map(|x| &x.tok), Some(Tok::Ident(_))) ||
            t.get(i+7).map(|x| &x.tok) != Some(&Tok::Pipe) ||
            t.get(i+8).map(|x| &x.tok) != Some(&Tok::Arrow) {
            return Err(fail(t[i+3].pos, "E_UNSUPPORTED",
                "suspending direct lambda currently requires |int argument| -> int-expression"));
        }
        let Tok::Ident(param) = &t[i+6].tok else { unreachable!() };
        if param == binding { return Err(fail(t[i+6].pos, "E_DUPLICATE",
            "lambda parameter shadows its binding")); }
        let mut j = i + 9;
        let (mut parentheses, mut brackets) = (0usize, 0usize);
        while j < t.len() {
            match t[j].tok {
                Tok::LParen => parentheses += 1,
                Tok::RParen => {
                    if parentheses == 0 { return Err(fail(t[j].pos, "E_SYNTAX", "unmatched lambda ')'")); }
                    parentheses -= 1;
                }
                Tok::LBracket => brackets += 1,
                Tok::RBracket => {
                    if brackets == 0 { return Err(fail(t[j].pos, "E_SYNTAX", "unmatched lambda ']'")); }
                    brackets -= 1;
                }
                Tok::LBrace | Tok::RBrace => return Err(fail(t[j].pos, "E_UNSUPPORTED",
                    "suspending direct lambda requires a single expression body")),
                Tok::Semi if parentheses == 0 && brackets == 0 => break,
                Tok::Eof => break,
                _ => {}
            }
            j += 1;
            if j - i > 512 {
                return Err(fail(t[i].pos, "E_SOURCE_LIMIT", "suspending lambda expression token budget"));
            }
        }
        if t.get(j).map(|x| &x.tok) != Some(&Tok::Semi) || j == i+9 {
            return Err(fail(t[i].pos, "E_SYNTAX", "unterminated suspending lambda expression"));
        }
        if parentheses != 0 || brackets != 0 {
            return Err(fail(t[i].pos, "E_SYNTAX", "unbalanced suspending lambda expression"));
        }
        found = Some((i, j+1, scope_open, binding.clone(), param.clone(),
            t[i+9..j].to_vec(), t[i].pos));
        i = j+1;
    }

    let Some((start, end, scope, binding, param, body, pos)) = found else {
        // A `suspends` modifier on a function or method is handled directly by
        // the typed parser. An unrecognized lambda expression remains rejected.
        return Ok(t);
    };
    let mut closing = None;
    let mut braces = 0usize;
    for (j, token) in t.iter().enumerate().skip(scope) {
        if token.tok == Tok::LBrace { braces += 1; }
        if token.tok == Tok::RBrace {
            braces -= 1;
            if braces == 0 { closing = Some(j); break; }
        }
    }
    let close = closing.ok_or_else(|| fail(pos, "E_EOF", "unclosed lambda owner function"))?;
    if end > close { return Err(fail(pos, "E_SYNTAX", "lambda body exceeds function scope")); }

    let hidden = "$atto_susp_lambda_0".to_string();
    let mut result = Vec::with_capacity(t.len()+body.len()+18);
    let mut call_count = 0usize;
    for j in 0..t.len() {
        if j >= start && j < end { continue; }
        if word(&t, j, &binding) {
            let is_call = t.get(j+1).map(|x| &x.tok) == Some(&Tok::LParen)
                && (j == 0 || t[j-1].tok != Tok::Dot);
            if j <= scope || j >= close || !is_call ||
                (j > 0 && (word(&t, j-1, "fnc") || word(&t, j-1, "method") ||
                    word(&t, j-1, "mtd") || word(&t, j-1, "routine"))) {
                return Err(fail(t[j].pos, "E_UNSUPPORTED",
                    "suspending lambda cannot escape, be shadowed, or be called outside lexical owner"));
            }
            call_count += 1;
            result.push(tok(Tok::Ident(hidden.clone()), t[j].pos));
        } else {
            result.push(t[j].clone());
        }
    }
    if call_count == 0 {
        return Err(fail(pos, "E_UNSUPPORTED", "unused suspending lambda requires verified owned callable storage"));
    }
    let eof = result.pop().ok_or_else(|| fail(pos,"E_EOF","empty source"))?;
    if eof.tok != Tok::Eof { return Err(fail(pos,"E_SYNTAX","missing source EOF")); }
    result.extend([
        tok(Tok::Ident("suspends".into()), pos),
        tok(Tok::Ident("fnc".into()), pos),
        tok(Tok::Ident(hidden), pos),
        tok(Tok::LParen, pos), tok(Tok::Ident("int".into()), pos),
        tok(Tok::Ident(param), pos), tok(Tok::RParen, pos),
        tok(Tok::Colon, pos), tok(Tok::Ident("int".into()), pos),
        tok(Tok::LBrace, pos), tok(Tok::Ident("return".into()), pos),
    ]);
    result.extend(body);
    result.push(tok(Tok::Semi, pos));
    result.push(tok(Tok::RBrace, pos));
    result.push(eof);
    if result.len() > crate::lexer::MAX_TOKENS {
        return Err(fail(pos, "E_SOURCE_LIMIT", "suspending lambda expansion exceeds token limit"));
    }
    Ok(result)
}

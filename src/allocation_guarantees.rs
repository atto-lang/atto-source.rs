//! Conservative, actual typed-IR allocation/copy proof for closed scalar modules.
//! No annotations are discarded by an alternate lowering path.
use crate::{ast::{Type, BinOp}, check::{TExpr, TStmt, TE}, lexer::{Tok, Token}, Diagnostic, Pos};

fn marker(t: &Tok) -> Option<&'static str> {
    match t {
        Tok::Ident(s) if s == "noalloc" => Some("E_NOALLOC"),
        Tok::Ident(s) if s == "nocopy" => Some("E_NOCOPY"),
        _ => None,
    }
}
pub(crate) fn present(t: &[Token]) -> bool {
    t.windows(2).any(|w| w[0].tok == Tok::At && marker(&w[1].tok).is_some())
}
fn scalar(t: Type) -> bool { matches!(t, Type::Int | Type::Bool) }
fn expr(e: &TE, proven: &[bool]) -> bool {
    scalar(e.ty) && match &e.e {
        TExpr::I64(_) | TExpr::Bool(_) | TExpr::Local(_) => true,
        // Direct scalar opcodes only. Checked arithmetic helpers and
        // additional effectful IR remain outside this proof revision.
        TExpr::Bin(op, a, b) => matches!(op, BinOp::BitAnd | BinOp::BitOr |
            BinOp::BitXor | BinOp::Shl | BinOp::LShr | BinOp::AShr |
            BinOp::Eq | BinOp::Ne | BinOp::Lt | BinOp::Le | BinOp::Gt |
            BinOp::Ge | BinOp::And | BinOp::Or) && expr(a, proven) && expr(b, proven),
        TExpr::Not(a) => expr(a, proven),
        TExpr::Ternary(c, a, b) => expr(c, proven) && expr(a, proven) && expr(b, proven),
        TExpr::Call(target, args) => proven.get(*target as usize) == Some(&true)
            && args.iter().all(|a| expr(a, proven)),
        _ => false,
    }
}
pub(crate) fn compile(tokens: Vec<Token>) -> Result<Vec<u8>, Diagnostic> {
    let mut plain = Vec::with_capacity(tokens.len());
    let mut i = 0;
    let mut depth = 0usize;
    let mut obligations = Vec::new();
    while i < tokens.len() {
        if tokens[i].tok == Tok::At {
            let pos = tokens[i].pos;
            if depth != 0 || !plain.last().map_or(true, |t: &Token| t.tok == Tok::RBrace) {
                return Err(Diagnostic::new(pos, "E_ANNOTATION", "guarantee must precede a top-level brace callable"));
            }
            let mut seen = [false; 2];
            while tokens.get(i).is_some_and(|t| t.tok == Tok::At) {
                let code = tokens.get(i+1).and_then(|t| marker(&t.tok))
                    .ok_or_else(|| Diagnostic::new(pos, "E_ANNOTATION", "unsupported annotation in guarantee profile"))?;
                let index = usize::from(code == "E_NOCOPY");
                if seen[index] { return Err(Diagnostic::new(pos, "E_ANNOTATION", "duplicate guarantee")); }
                seen[index] = true;
                obligations.push((pos, code));
                i += 2;
            }
            let mut j = i;
            let word = |n| matches!(tokens.get(n), Some(Token {tok: Tok::Ident(_), ..}));
            let is = |n, s: &str| matches!(tokens.get(n), Some(Token {tok: Tok::Ident(w), ..}) if w == s);
            if is(j, "pub") { j += 1; }
            if is(j, "define") { j += 1; if is(j, "pub") { j += 1; } }
            if !word(j) || !is(j, "fnc") {
                return Err(Diagnostic::new(pos, "E_ANNOTATION", "guarantees require a named fnc"));
            }
            continue;
        }
        match tokens[i].tok {
            Tok::LBrace => depth += 1,
            Tok::RBrace => depth = depth.saturating_sub(1),
            _ => {}
        }
        plain.push(tokens[i].clone());
        i += 1;
    }
    let (pos, code) = obligations.first().copied().unwrap_or((Pos {line:1,col:1}, "E_NOALLOC"));
    let deny = || Diagnostic::new(pos, code, "cannot prove all reachable allocation/copy effects in closed scalar profile");
    let program = crate::parser::Parser::new(plain).parse_program()?;
    let typed = crate::check::check(&program)?;
    // Deliberately prove the entire module in v1, stronger than callable scope.
    // Deny every helper, host import, object, string, continuation or unwind ABI.
    if typed.heap || typed.strings || typed.guest_eh || typed.raise_tag || typed.catch_depth ||
       !typed.imports.is_empty() || !typed.data.is_empty() || !typed.globals.is_empty() ||
       !typed.actors.is_empty() || !typed.generators.is_empty() { return Err(deny()); }
    let mut proven = vec![false; typed.funcs.len()];
    // Monotone bounded fixed point: cycles and unknown targets never become proven.
    for _ in 0..typed.funcs.len() {
        let mut changed = false;
        for (idx, func) in typed.funcs.iter().enumerate() {
            if !proven[idx] && scalar(func.ret) && func.params.iter().copied().all(scalar) &&
               func.locals.is_empty() && matches!(func.body.as_slice(), [TStmt::Return(Some(e))] if expr(e, &proven)) {
                proven[idx] = true;
                changed = true;
            }
        }
        if !changed { break; }
    }
    if !proven.iter().all(|v| *v) { return Err(deny()); }
    crate::actor_stack_bound::verify(&typed)?;
    let bytes = crate::wasm::emit(&typed);
    if bytes.len() > crate::wasm::MAX_OUTPUT {
        return Err(Diagnostic::new(pos, "E_OUTPUT_LIMIT", "generated module exceeds 1 MiB"));
    }
    if !memoryless_sections(&bytes) {
        return Err(Diagnostic::new(pos, code, "emitted scalar module violates memoryless guarantee envelope"));
    }
    Ok(bytes)
}

// Defense in depth: require the actual emitted binary to have no imports,
// linear memory, tables, element/data segments, start function or tags.
// This is NOT a general Wasm verifier or an independent all-path proof.
fn memoryless_sections(bytes: &[u8]) -> bool {
    if !bytes.starts_with(b"\0asm\x01\0\0\0") { return false; }
    let mut i = 8usize;
    let mut seen = [false; 13];
    let mut previous = 0u8;
    while i < bytes.len() {
        let id = bytes[i]; i += 1;
        // The closed scalar emitter needs only these core sections.
        if !matches!(id, 0 | 1 | 3 | 6 | 7 | 10) { return false; }
        if id != 0 {
            if id <= previous || seen[id as usize] { return false; }
            previous = id;
            seen[id as usize] = true;
        }
        let mut size = 0u32;
        let mut complete = false;
        for shift in (0..35).step_by(7) {
            let Some(&b) = bytes.get(i) else { return false; };
            i += 1;
            if shift == 28 && b > 0x0f { return false; }
            size |= u32::from(b & 0x7f) << shift;
            if b & 0x80 == 0 { complete = true; break; }
        }
        if !complete { return false; }
        let Some(end) = i.checked_add(size as usize) else { return false; };
        if end > bytes.len() { return false; }
        i = end;
    }
    seen[1] && seen[3] && seen[7] && seen[10]
}

#[cfg(test)]
mod tests {
    use super::memoryless_sections;
    #[test]
    fn rejects_forbidden_sections_and_truncation() {
        let actual = crate::compile("@noalloc\n@nocopy\npub fnc main():int { return 42; }").unwrap();
        assert!(memoryless_sections(&actual));
        for id in [2, 4, 5, 8, 9, 11, 12, 13] {
            let mut altered = actual.clone();
            altered.extend([id, 0]);
            assert!(!memoryless_sections(&altered), "forbidden section {id}");
        }
        for suffix in [&[0, 0x80][..], &[0, 0xff, 0xff, 0xff, 0xff, 0x10][..], &[0, 10, 1][..]] {
            let mut altered = actual.clone(); altered.extend(suffix);
            assert!(!memoryless_sections(&altered));
        }
    }
}

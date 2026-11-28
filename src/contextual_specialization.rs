//! Initial executable contextual specialization profile.
//! One source-local nominal class, one public contextual static method, one
//! Copy scalar context (int/bool), and straight-line main. Lower to ordinary
//! checked static dispatch and immutable locals. No capability capture yet.
use crate::diag::Diagnostic;
use crate::lexer::{Tok, Token, MAX_TOKENS};
use std::collections::BTreeMap;

const PREFIX: &str = "_atto_context_v1_";
fn word(t: &Token) -> Option<&str> { if let Tok::Ident(s) = &t.tok { Some(s) } else { None } }
fn w(t: &[Token], i: usize) -> Option<&str> { t.get(i).and_then(word) }
fn is(t: &[Token], i: usize, k: Tok) -> bool { t.get(i).is_some_and(|x| x.tok == k) }
fn err(t: &Token, msg: &str) -> Diagnostic { Diagnostic::new(t.pos, "E_CONTEXT_PROFILE", msg) }
fn token(t: &Token, tok: Tok) -> Token { Token { tok, pos: t.pos } }
fn ident(t: &Token, s: &str) -> Token { token(t, Tok::Ident(s.into())) }
fn pair(t: &[Token], i: usize, open: Tok, close: Tok) -> Result<usize, Diagnostic> {
    if !is(t, i, open.clone()) { return Err(err(&t[i.min(t.len()-1)], "expected contextual delimiter")); }
    let mut d = 0;
    for (j, x) in t.iter().enumerate().skip(i) {
        if x.tok == open { d += 1; }
        if x.tok == close { d -= 1; if d == 0 { return Ok(j); } }
    }
    Err(err(&t[i], "unclosed contextual delimiter"))
}
pub fn present(t: &[Token]) -> bool {
    // The existing callable body separator also accepts `-> int :: (expr) end`.
    // Activate this bounded lowerer only for a contextual declaration; ordinary
    // callable bodies must remain on the original parser path.
    t.windows(4).any(|x| word(&x[0]) == Some("fnc")
        && word(&x[1]).is_some() && x[2].tok == Tok::ColonColon
        && x[3].tok == Tok::LParen)
}
fn plain(t: &[Token], owner: &str, aliases: &BTreeMap<String,String>, anchor: &Token, context: bool) -> Result<(), Diagnostic> {
    if context && t.is_empty() { return Err(err(anchor, "exactly one context expression is required")); }
    // Track delimiter kind as well as depth before splicing context tokens
    // into a different argument list. A mismatched close cannot hide a comma.
    let mut delimiters = Vec::new();
    for x in t {
        if word(x).is_some_and(|s| s == owner || aliases.contains_key(s)) ||
            matches!(x.tok, Tok::ColonColon | Tok::LBrace | Tok::RBrace | Tok::Pipe | Tok::FatArrow | Tok::Semi) {
            return Err(err(x, "nested contextual binding/call or capture is outside scalar profile"));
        }
        match x.tok {
            Tok::LParen => delimiters.push(Tok::RParen),
            Tok::LBracket => delimiters.push(Tok::RBracket),
            Tok::RParen | Tok::RBracket => {
                if delimiters.pop().as_ref() != Some(&x.tok) {
                    return Err(err(x, "mismatched contextual expression delimiter"));
                }
            }
            Tok::Comma | Tok::Assign if context && delimiters.is_empty() => return Err(err(x, "one positional context expression required")),
            _ => {}
        }
    }
    if !delimiters.is_empty() {
        return Err(err(anchor, "unclosed contextual expression delimiter"));
    }
    Ok(())
}

pub fn lower(t: Vec<Token>) -> Result<Vec<Token>, Diagnostic> {
    if !present(&t) { return Ok(t); }
    let bad = |i: usize, msg: &str| err(&t[i.min(t.len()-1)], msg);
    if t.iter().any(|x| word(x).is_some_and(|s| s.starts_with(PREFIX))) {
        return Err(bad(0, "reserved compiler contextual name"));
    }
    // Deliberately admit one complete, source-local profile rather than
    // rewrite ambiguous names across modules, scopes or imported declarations.
    if w(&t,0) != Some("define") || w(&t,1) != Some("class") || w(&t,3) != Some("as")
        || w(&t,4) != Some("pub") || w(&t,5) != Some("static") || w(&t,6) != Some("fnc")
        || !is(&t,8,Tok::ColonColon) {
        return Err(bad(0, "contextual scalar profile requires one class with a public static contextual method"));
    }
    let owner = w(&t,2).ok_or_else(|| bad(2,"nominal owner required"))?;
    let member = w(&t,7).ok_or_else(|| bad(7,"method name required"))?;
    if crate::parser::is_reserved(owner) || crate::parser::is_reserved(member) || matches!(member, "new" | "make") {
        return Err(bad(7,"invalid contextual owner or member name"));
    }
    let ctx_end = pair(&t,9,Tok::LParen,Tok::RParen)?;
    if ctx_end != 12 || !matches!(w(&t,10),Some("int" | "bool")) || w(&t,11).is_none() {
        return Err(bad(9,"only one typed Copy scalar context parameter (int or bool) is implemented"));
    }
    let ctx_type = w(&t,10).unwrap();
    let ctx_name = w(&t,11).unwrap();
    if crate::parser::is_reserved(ctx_name) || ctx_name == owner { return Err(bad(11,"invalid context parameter")); }
    let params_end = pair(&t,13,Tok::LParen,Tok::RParen)?;
    let body_at = params_end + 3;
    if !(is(&t,params_end+1,Tok::Arrow) || is(&t,params_end+1,Tok::Colon))
        || !matches!(w(&t,params_end+2),Some("int" | "bool")) {
        return Err(bad(params_end,"explicit scalar result required"));
    }
    let body_end = pair(&t,body_at,Tok::LBrace,Tok::RBrace)?;
    if w(&t,body_end+1) != Some("end") || w(&t,body_end+2) != Some("fnc") || w(&t,body_end+3) != Some("main")
        || !is(&t,body_end+4,Tok::LParen) || !is(&t,body_end+5,Tok::RParen)
        || !(is(&t,body_end+6,Tok::Arrow) || is(&t,body_end+6,Tok::Colon))
        || w(&t,body_end+7) != Some("int") {
        return Err(bad(body_end,"contextual scalar profile requires class end followed by fnc main() -> int"));
    }
    let main_at = body_end+8;
    let main_end = pair(&t,main_at,Tok::LBrace,Tok::RBrace)?;
    if main_end+2 != t.len() || !is(&t,main_end+1,Tok::Eof) {
        return Err(bad(main_end,"additional declarations are not admitted in this contextual profile"));
    }
    // Neither the method body nor ordinary parameters may reference the
    // original owner; recursive or qualified contextual dispatch is not yet supported.
    if t[13..=body_end].iter().any(|x| word(x)==Some(owner) || x.tok==Tok::ColonColon) {
        return Err(bad(13,"contextual member body must not rebind or recursively specialize owner"));
    }
    let lowered = format!("{PREFIX}method");
    let mut out = t[..7].to_vec();
    out.push(ident(&t[7],&lowered));
    out.push(t[13].clone());
    out.extend_from_slice(&t[10..12]);
    if params_end != 14 { out.push(token(&t[13],Tok::Comma)); }
    out.extend_from_slice(&t[14..=main_at]);
    let mut aliases = BTreeMap::<String,String>::new();
    let mut i = main_at+1;
    while i < main_end {
        let x = &t[i];
        if matches!(x.tok, Tok::LBrace | Tok::RBrace | Tok::Pipe | Tok::FatArrow)
            || word(x).is_some_and(|s| matches!(s,"if"|"for"|"while"|"loop"|"do"|"match"|"switch"|"try"|"defer"|"recover"|"select"|"fnc"|"generator"|"block"|"then"|"else"|"end"|"async"|"suspend"|"using")) {
            return Err(bad(i,"nested scopes and control flow are outside contextual scalar profile"));
        }
        // Immutable stored binding. The source alias has no scalar value
        // representation; only its generated, typed capture slot does.
        if w(&t,i)==Some("const") && is(&t,i+2,Tok::Assign) && w(&t,i+3)==Some(owner)
            && is(&t,i+4,Tok::ColonColon) && is(&t,i+5,Tok::LParen) {
            let end = pair(&t,i+5,Tok::LParen,Tok::RParen)?;
            if is(&t,end+1,Tok::Semi) {
                let name = w(&t,i+1).ok_or_else(|| bad(i,"context binding name required"))?;
                if crate::parser::is_reserved(name) || name==owner || aliases.contains_key(name)
                    || t[main_at+1..i].iter().any(|q| word(q)==Some(name)) {
                    return Err(bad(i,"context binding conflicts with a prior source name"));
                }
                if i!=main_at+1 && !is(&t,i-1,Tok::Semi) { return Err(bad(i,"context binding must be a standalone statement")); }
                plain(&t[i+6..end],owner,&aliases,x,true)?;
                if t[i+6..end].iter().any(|q| word(q)==Some(name)) { return Err(bad(i,"self-referencing contextual binding")); }
                if aliases.len()>=64 { return Err(bad(i,"context binding limit (64)")); }
                let slot = format!("{PREFIX}slot_{}",aliases.len());
                out.extend([x.clone(),ident(&t[i+1],&slot),token(x,Tok::Colon),ident(x,ctx_type),t[i+2].clone()]);
                out.extend_from_slice(&t[i+6..end]);
                out.push(t[end+1].clone());
                aliases.insert(name.into(),slot);
                i=end+2; continue;
            }
        }
        let stored = word(x).and_then(|name| aliases.get(name));
        let direct = word(x)==Some(owner);
        if direct || stored.is_some() {
            // Rewriting a member segment or binding identifier into an
            // owner expression would change lexical resolution.
            if i>main_at+1 && (is(&t,i-1,Tok::Dot) || is(&t,i-1,Tok::ColonColon)) {
                return Err(bad(i,"contextual receiver must be an unqualified nominal or local"));
            }
            let (dot, context) = if direct {
                if !is(&t,i+1,Tok::ColonColon) { return Err(bad(i,"contextual method requires explicit binding")); }
                let end=pair(&t,i+2,Tok::LParen,Tok::RParen)?;
                plain(&t[i+3..end],owner,&aliases,x,true)?;
                (end+1,t[i+3..end].to_vec())
            } else {
                (i+1,vec![ident(x,stored.unwrap())])
            };
            if !is(&t,dot,Tok::Dot) || w(&t,dot+1)!=Some(member) || !is(&t,dot+2,Tok::LParen) {
                return Err(bad(i,"bound context is only usable through its declared static member"));
            }
            let end=pair(&t,dot+2,Tok::LParen,Tok::RParen)?;
            if end>=main_end { return Err(bad(i,"context call crosses main boundary")); }
            plain(&t[dot+3..end],owner,&aliases,x,false)?;
            out.extend([ident(x,owner),t[dot].clone(),ident(&t[dot+1],&lowered),t[dot+2].clone()]);
            out.extend(context);
            if end!=dot+3 { out.push(token(x,Tok::Comma)); }
            out.extend_from_slice(&t[dot+3..=end]);
            i=end+1; continue;
        }
        if x.tok==Tok::ColonColon { return Err(bad(i,"unsupported contextual specialization")); }
        out.push(x.clone()); i+=1;
    }
    out.extend_from_slice(&t[main_end..]);
    if out.len()>MAX_TOKENS { return Err(bad(0,"contextual lowering token budget exceeded")); }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn program(body: &str) -> String {
        format!("define class Scale as pub static fnc apply::(int factor)(int value) -> int {{ return factor * value; }} end fnc main() -> int {{ {body} }}")
    }
    #[test] fn ordinary_input_unchanged() {
        let t=crate::lexer::lex("fnc main() -> int { return 7; }").unwrap();
        assert!(!present(&t)); assert_eq!(lower(t.clone()).unwrap().len(),t.len());
    }
    #[test] fn legacy_callable_separator_does_not_activate_context_lowering() {
        let t = crate::lexer::lex("fnc main() -> int :: (42) end").unwrap();
        assert!(!present(&t));
        let original: Vec<_> = t.iter().map(|x| x.tok.clone()).collect();
        let lowered: Vec<_> = lower(t).unwrap().into_iter().map(|x| x.tok).collect();
        assert_eq!(original, lowered);
    }
    #[test] fn stored_capture_and_direct_compile() {
        let src=program("let n = 3; const x = Scale::(n); n = 9; const a = x.apply(4); const b = x.apply(5); return a + b + Scale::(2).apply(7);");
        let bytes=crate::compile(&src).expect("checked contextual guest compilation");
        assert_eq!(&bytes[..4],b"\0asm");
    }
    // This gate executes the actual emitted Wasm, never a host reimplementation.
    // Run explicitly with Node.js installed: cargo test --lib contextual_specialization::tests::guest_capture -- --ignored
    #[test]
    #[ignore = "requires Node.js guest runtime"]
    fn guest_capture() {
        use std::io::Write;
        use std::process::{Command, Stdio};
        for (body, expected) in [
            ("let n = 3; const x = Scale::(n); n = 9; const a = x.apply(4); const b = x.apply(5); return a + b + Scale::(2).apply(7);", "41"),
            ("const x = Scale::(2); const y = Scale::(3); return x.apply(4) + y.apply(4);", "20"),
            ("return Scale::(6).apply(7);", "42"),
            ("let n = 3; const x = Scale::(n + 1); n = 9; return x.apply(n) + Scale::(n - 7).apply(3);", "42"),
        ] {
            let wasm = crate::compile(&program(body)).expect("compile guest");
            let mut child = Command::new("node").args(["-e",
                "const fs=require('fs');const m=new WebAssembly.Module(fs.readFileSync(0));const i=new WebAssembly.Instance(m,{});process.stdout.write(String(i.exports.main()));"])
                .stdin(Stdio::piped()).stdout(Stdio::piped()).spawn().expect("Node.js required");
            child.stdin.take().unwrap().write_all(&wasm).unwrap();
            let result = child.wait_with_output().unwrap();
            assert!(result.status.success(), "guest failed: {body}");
            assert_eq!(String::from_utf8(result.stdout).unwrap(), expected);
        }
    }
    #[test] fn reject_mismatched_context_delimiters_before_rewrite() {
        for body in [
            "return Scale::([2).apply(3);",
            "return Scale::(2]).apply(3);",
            "return Scale::(2).apply([3);",
            "const x = Scale::(2]); return x.apply(3);",
        ] {
            let tokens = crate::lexer::lex(&program(body)).expect("lexical tokens");
            assert!(lower(tokens).is_err(), "{body}");
        }
    }
    #[test] fn contextual_capture_count_is_bounded() {
        use std::fmt::Write as _;
        let mut body = String::new();
        for i in 0..64 { write!(&mut body, "const cap{i} = Scale::(2); ").unwrap(); }
        let accepted = program(&format!("{body}return cap63.apply(3);"));
        assert!(lower(crate::lexer::lex(&accepted).unwrap()).is_ok());
        let denied = program(&format!("{body}const extra = Scale::(2); return extra.apply(3);"));
        assert!(lower(crate::lexer::lex(&denied).unwrap()).is_err());
    }
    #[test] fn separate_bindings_compile() {
        assert!(crate::compile(&program("const x = Scale::(2); const y = Scale::(3); return x.apply(4) + y.apply(4);")).is_ok());
    }
    #[test] fn reject_malformed_and_unsupported() {
        for body in [
            "const x = Scale::(1, 2); return x.apply(3);",
            "const x = Scale::(true); return x.apply(3);",
            "return Scale.apply(3);",
            "const x = Scale::(2); return x;",
            "const x = Scale::(2); x = 9; return x.apply(3);",
            "const x = Scale::(2); return x.other(3);",
            "const x = Scale::(2); const x = Scale::(3); return x.apply(3);",
            "let x = 1; const x = Scale::(2); return x.apply(3);",
            "const x = Scale::(2); return x.apply(true);",
            "return Scale::(1, 2).apply(3);",
            "return Scale::(2).apply();",
            "const x = Scale::(2); return _atto_context_v1_slot_0;",
        ] { assert!(crate::compile(&program(body)).is_err(),"{body}"); }
    }
}

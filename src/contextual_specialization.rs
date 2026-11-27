//! Initial executable contextual specialization profile.
//! One source-local nominal class, one public contextual static method, one
//! to eight Copy scalar contexts (int/bool), and straight-line main. Lower to ordinary
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
// Split only at top-level commas, preserving source order and nested calls.
fn split_context<'a>(t: &'a [Token], anchor: &Token) -> Result<Vec<&'a [Token]>, Diagnostic> {
    let mut parts = Vec::new();
    let mut stack = Vec::new();
    let mut start = 0;
    for (i, x) in t.iter().enumerate() {
        match x.tok {
            Tok::LParen => stack.push(Tok::RParen),
            Tok::LBracket => stack.push(Tok::RBracket),
            Tok::RParen | Tok::RBracket => {
                if stack.pop().as_ref() != Some(&x.tok) {
                    return Err(err(x, "mismatched contextual delimiter"));
                }
            }
            Tok::Comma if stack.is_empty() => {
                if i == start { return Err(err(x, "empty context argument")); }
                parts.push(&t[start..i]);
                start = i+1;
            }
            _ => {}
        }
    }
    if !stack.is_empty() || start == t.len() {
        return Err(err(anchor, "empty or unclosed context argument"));
    }
    parts.push(&t[start..]);
    if parts.len() > 8 { return Err(err(anchor, "context arity limit (8)")); }
    Ok(parts)
}

fn plain(t: &[Token], owner: &str, aliases: &BTreeMap<String,Vec<String>>, anchor: &Token, context: bool) -> Result<(), Diagnostic> {
    if context && t.is_empty() { return Err(err(anchor, "exactly one context expression is required")); }
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
    let mut schema = Vec::new();
    for decl in split_context(&t[10..ctx_end], &t[9])? {
        if decl.len() != 2 || !matches!(word(&decl[0]), Some("int" | "bool")) {
            return Err(err(&decl[0], "context parameters must be typed Copy scalars (int or bool)"));
        }
        let ty = word(&decl[0]).unwrap();
        let name = word(&decl[1]).ok_or_else(|| err(&decl[1], "context parameter name required"))?;
        if crate::parser::is_reserved(name) || name == owner || schema.iter().any(|&(_, n)| n == name) {
            return Err(err(&decl[1], "invalid or duplicate context parameter"));
        }
        schema.push((ty, name));
    }
    let params_at = ctx_end + 1;
    let params_end = pair(&t,params_at,Tok::LParen,Tok::RParen)?;
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
    if t[params_at..=body_end].iter().any(|x| word(x)==Some(owner) || x.tok==Tok::ColonColon) {
        return Err(bad(params_at,"contextual member body must not rebind or recursively specialize owner"));
    }
    let lowered = format!("{PREFIX}method");
    let mut out = t[..7].to_vec();
    out.push(ident(&t[7],&lowered));
    out.push(t[params_at].clone());
    out.extend_from_slice(&t[10..ctx_end]);
    if params_end != params_at + 1 { out.push(token(&t[params_at],Tok::Comma)); }
    out.extend_from_slice(&t[params_at+1..=main_at]);
    let mut aliases = BTreeMap::<String,Vec<String>>::new();
    let mut i = main_at+1;
    let mut main_return_seen = false;
    let mut writable = std::collections::BTreeSet::<String>::new();
    while i < main_end {
        let x = &t[i];
        // Closed direct named return: no hoisting across a sibling expression.
        // Capture RHSs in source order, then invoke with declaration-order slots.
        if word(x)==Some("return") && w(&t,i+1)==Some(owner)
            && is(&t,i+2,Tok::ColonColon) && is(&t,i+3,Tok::LParen) {
            let close=pair(&t,i+3,Tok::LParen,Tok::RParen)?;
            let parts=split_context(&t[i+4..close],x)?;
            if parts.iter().any(|p| is(p,1,Tok::Assign)) {
                if main_return_seen || (i!=main_at+1 && !is(&t,i-1,Tok::Semi))
                    || parts.len()!=schema.len() { return Err(bad(i,"invalid direct named return")); }
                let dot=close+1;
                if !is(&t,dot,Tok::Dot)||w(&t,dot+1)!=Some(member)||!is(&t,dot+2,Tok::LParen) {
                    return Err(bad(i,"expected declared contextual member"));
                }
                let end=pair(&t,dot+2,Tok::LParen,Tok::RParen)?;
                if end+2!=main_end || !is(&t,end+1,Tok::Semi) {
                    return Err(bad(i,"direct named invocation requires entire terminal return"));
                }
                let mut slots=vec![String::new();schema.len()];
                for part in parts {
                    if !is(part,1,Tok::Assign) { return Err(bad(i,"mixed named and positional context")); }
                    let key=w(part,0).ok_or_else(||bad(i,"named context key required"))?;
                    let j=schema.iter().position(|&(_,n)|n==key).ok_or_else(||bad(i,"unknown named context key"))?;
                    if !slots[j].is_empty() {return Err(bad(i,"duplicate named context key"));}
                    plain(&part[2..],owner,&aliases,x,true)?;
                    let slot=format!("{PREFIX}direct_{}_{}",i,j);
                    out.extend([ident(x,"const"),ident(x,&slot),token(x,Tok::Colon),ident(x,schema[j].0),token(x,Tok::Assign)]);
                    out.extend_from_slice(&part[2..]);out.push(token(x,Tok::Semi));slots[j]=slot;
                }
                plain(&t[dot+3..end],owner,&aliases,x,false)?;
                out.extend([x.clone(),ident(x,owner),t[dot].clone(),ident(x,&lowered),t[dot+2].clone()]);
                for (j,slot) in slots.iter().enumerate(){if j>0{out.push(token(x,Tok::Comma));}out.push(ident(x,slot));}
                if end!=dot+3{out.push(token(x,Tok::Comma));}
                out.extend_from_slice(&t[dot+3..=end+1]);
                i=end+2;continue;
            }
        }
        if word(x)==Some("return") { main_return_seen = true; }
        if matches!(x.tok, Tok::LBrace | Tok::RBrace | Tok::Pipe | Tok::FatArrow)
            || word(x).is_some_and(|s| matches!(s,"if"|"for"|"while"|"loop"|"do"|"match"|"switch"|"try"|"defer"|"recover"|"select"|"fnc"|"generator"|"block"|"then"|"else"|"end"|"async"|"suspend"|"using")) {
            return Err(bad(i,"nested scopes and control flow are outside contextual scalar profile"));
        }
        // Explicit set is a standalone reassignment of an existing scalar let.
        // Only the already checked legacy assignment lowering is reused.
        let boundary = i==main_at+1 || is(&t,i-1,Tok::Semi);
        if boundary && w(&t,i)==Some("let") && is(&t,i+2,Tok::Assign) {
            let name=w(&t,i+1).ok_or_else(|| bad(i,"scalar let name required"))?;
            if aliases.contains_key(name) || name==owner { return Err(bad(i,"scalar let conflicts with contextual binding")); }
            writable.insert(name.into());
        }
        if boundary && w(&t,i)==Some("set") {
            let name=w(&t,i+1).ok_or_else(|| bad(i,"set target required"))?;
            if main_return_seen || !writable.contains(name) || aliases.contains_key(name) || !is(&t,i+2,Tok::Assign) {
                return Err(bad(i,"set requires an existing writable scalar binding"));
            }
            i+=1; continue;
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
                let parts = split_context(&t[i+6..end], x)?;
                if parts.len() != schema.len() { return Err(bad(i, "context arity mismatch")); }
                if t[i+6..end].iter().any(|q| word(q)==Some(name)) { return Err(bad(i,"self-referencing contextual binding")); }
                if aliases.len()>=64 { return Err(bad(i,"context binding limit (64)")); }
                let named = parts.iter().any(|part| is(part,1,Tok::Assign));
                let mut slots = vec![String::new();schema.len()];
                for (k, part) in parts.into_iter().enumerate() {
                    let (j, value) = if named {
                        if !is(part,1,Tok::Assign) { return Err(bad(i,"mixed named and positional context")); }
                        let key = w(part,0).ok_or_else(|| bad(i,"named context key required"))?;
                        let j = schema.iter().position(|&(_,n)| n==key)
                            .ok_or_else(|| bad(i,"unknown named context key"))?;
                        if !slots[j].is_empty() { return Err(bad(i,"duplicate named context key")); }
                        (j,&part[2..])
                    } else { (k,part) };
                    plain(value,owner,&aliases,x,true)?;
                    let slot = format!("{PREFIX}slot_{}_{}",aliases.len(),j);
                    out.extend([x.clone(),ident(&t[i+1],&slot),token(x,Tok::Colon),ident(x,schema[j].0),t[i+2].clone()]);
                    out.extend_from_slice(value);
                    out.push(t[end+1].clone());
                    slots[j]=slot;
                }
                aliases.insert(name.into(),slots);
                i=end+2; continue;
            }
        }
        // Copying this closed scalar-only binding preserves the captured
        // immutable values. No context expression is re-evaluated and no
        // new runtime storage is required. Resource captures remain denied.
        if w(&t,i)==Some("const") && is(&t,i+2,Tok::Assign) && is(&t,i+4,Tok::Semi) {
            if main_return_seen && w(&t,i+3).is_some_and(|name| aliases.contains_key(name)) {
                return Err(bad(i,"context alias cannot follow terminating return"));
            }
            if let Some(slots) = w(&t,i+3).and_then(|name| aliases.get(name)).cloned() {
                let name = w(&t,i+1).ok_or_else(|| bad(i,"context alias name required"))?;
                if crate::parser::is_reserved(name) || name==owner || aliases.contains_key(name)
                    || t[main_at+1..i].iter().any(|q| word(q)==Some(name)) {
                    return Err(bad(i,"context alias conflicts with a prior source name"));
                }
                if aliases.len()>=64 || (i!=main_at+1 && !is(&t,i-1,Tok::Semi)) {
                    return Err(bad(i,"context alias limit or invalid statement boundary"));
                }
                aliases.insert(name.into(),slots);
                i+=5; continue;
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
                let parts = split_context(&t[i+3..end], x)?;
                if parts.len() != schema.len() { return Err(bad(i, "context arity mismatch")); }
                for part in parts { plain(part,owner,&aliases,x,true)?; }
                (end+1,t[i+3..end].to_vec())
            } else {
                let mut captured = Vec::new();
                for (j, slot) in stored.unwrap().iter().enumerate() {
                    if j > 0 { captured.push(token(x, Tok::Comma)); }
                    captured.push(ident(x,slot));
                }
                (i+1,captured)
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
        for (source, expected) in [
            (program("let n = 3; const x = Scale::(n); n = 9; const a = x.apply(4); const b = x.apply(5); return a + b + Scale::(2).apply(7);"), "41"),
            (program("const x = Scale::(2); const y = Scale::(3); return x.apply(4) + y.apply(4);"), "20"),
            (program("return Scale::(6).apply(7);"), "42"),
            (program("const x = Scale::(6); const y = x; const z = y; return z.apply(7);"), "42"),
            (program("let n = 3; const x = Scale::(n + 1); n = 9; return x.apply(n) + Scale::(n - 7).apply(3);"), "42"),
            (multi_program("let n = 3; const x = Scale::(n, n + 1); n = 9; return x.apply(5) + Scale::(2, 1).apply(7);"), "34"),
            (multi_program("const x = Scale::(2, 3); const y = Scale::(4, 5); return x.apply(7) + y.apply(7);"), "50"),
            (multi_program("const x = Scale::(offset=7,factor=5); return x.apply(7);"), "42"),
            (multi_program("return Scale::(offset=7,factor=5).apply(7);"), "42"),
            (multi_program("let n = 3; const x = Scale::(offset=n+1,factor=n); set n = 9; return x.apply(5) + Scale::(2,1).apply(7);"), "34"),
        ] {
            let wasm = crate::compile(&source).expect("compile guest");
            let mut child = Command::new("node").args(["-e",
                "const fs=require('fs');const m=new WebAssembly.Module(fs.readFileSync(0));const i=new WebAssembly.Instance(m,{});process.stdout.write(String(i.exports.main()));"])
                .stdin(Stdio::piped()).stdout(Stdio::piped()).spawn().expect("Node.js required");
            child.stdin.take().unwrap().write_all(&wasm).unwrap();
            let result = child.wait_with_output().unwrap();
            assert!(result.status.success(), "guest failed: {source}");
            assert_eq!(String::from_utf8(result.stdout).unwrap(), expected);
        }
    }
    fn multi_program(body: &str) -> String {
        format!("define class Scale as pub static fnc apply::(int factor, int offset)(int value) -> int {{ return factor * value + offset; }} end fnc main() -> int {{ {body} }}")
    }
    #[test] fn multiple_contexts_compile() {
        let src = multi_program("let n = 3; const x = Scale::(n, n + 1); n = 9; return x.apply(5) + Scale::(2, 1).apply(7);");
        assert!(crate::compile(&src).is_ok());
    }
    #[test] fn multiple_contexts_reject() {
        for body in [
            "const x = Scale::(2); return x.apply(3);",
            "const x = Scale::(2, true); return x.apply(3);",
            "return Scale::(2, 1, 0).apply(3);",
            "return Scale::(2,).apply(3);",
            "return Scale::(,2).apply(3);",
            "const x = Scale::(factor=2, factor=1); return x.apply(3);",
            "const x = Scale::(2,1); set x = 9; return x.apply(3);",
            "set n = 9; return Scale::(2,1).apply(3);",
            "const x = Scale::(offset=2, 1); return x.apply(3);",
            "const x = Scale::(unknown=2, factor=1); return x.apply(3);",
            "return 1 + Scale::(offset=2, factor=1).apply(3);",
            "return Scale::(offset=2, factor=1).apply(3) + 1;",
            "return Scale::(offset=2, offset=1).apply(3);",
        ] { assert!(crate::compile(&multi_program(body)).is_err(), "{body}"); }
    }
    #[test] fn reject_mismatched_ordinary_arguments_before_rewrite() {
        for body in [
            "return Scale::(2).apply([3);",
            "return Scale::(2).apply(3]);",
            "const x = Scale::(2); return x.apply([3);",
        ] {
            let tokens = crate::lexer::lex(&program(body)).expect("lexical tokens");
            assert!(lower(tokens).is_err(), "{body}");
        }
    }
    #[test] fn bounded_context_schema_and_alias_count() {
        let header = (0..8).map(|i| format!("int c{i}")).collect::<Vec<_>>().join(", ");
        let args = (0..8).map(|_| "1").collect::<Vec<_>>().join(", ");
        let src = format!("define class Scale as pub static fnc apply::({header})(int value) -> int {{ return c0 + c7 + value; }} end fnc main() -> int {{ return Scale::({args}).apply(40); }}");
        assert!(crate::compile(&src).is_ok());
        let denied = src.replace(&header, &format!("{header}, int c8")).replace(&args, &format!("{args}, 1"));
        assert!(lower(crate::lexer::lex(&denied).unwrap()).is_err());
        let body: String = (0..64).map(|i| format!("const cap{i} = Scale::(2, 3); ")).collect();
        assert!(lower(crate::lexer::lex(&multi_program(&format!("{body}return cap63.apply(3);"))).unwrap()).is_ok());
        assert!(lower(crate::lexer::lex(&multi_program(&format!("{body}const extra = Scale::(2, 3); return extra.apply(3);"))).unwrap()).is_err());
    }
    #[test] fn immutable_context_aliases() {
        assert!(crate::compile(&program("const x = Scale::(6); const y = x; const z = y; return z.apply(7);")).is_ok());
        for body in [
            "const x = Scale::(6); const y = x; y = 3; return x.apply(7);",
            "const x = Scale::(6); const x = x; return x.apply(7);",
            "const x = Scale::(6); const y = x; return y;",
            "const x = Scale::(6); return x.apply(7); const y = x;",
        ] { assert!(crate::compile(&program(body)).is_err(), "{body}"); }
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
            "const x = Scale::(2); return _atto_context_v1_slot_0_0;",
        ] { assert!(crate::compile(&program(body)).is_err(),"{body}"); }
    }
}

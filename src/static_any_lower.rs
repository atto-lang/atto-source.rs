//! Bounded single-file monomorphization for an immutable, known-type Any.
//!
//! This is NOT dynamic tagged-union/Any runtime support. It only lowers a
//! provably monomorphic value and a single file-local function taking it, and
//! specializes type-pattern arms while preserving live runtime value guards.
//! Any unknown, mutable, escaped or heterogeneously passed Any rejects before
//! producing guest Wasm. No arbitrary type erasure or open-world dispatch.
use crate::diag::{Diagnostic, Pos};
use crate::lexer::{Tok, Token};
type R<T> = Result<T, Diagnostic>;

fn word(t: &[Token], i: usize, w: &str) -> bool {
    matches!(t.get(i).map(|v| &v.tok), Some(Tok::Ident(s)) if s == w)
}
fn fail(p: Pos, code: &'static str, m: impl Into<String>) -> Diagnostic {
    Diagnostic::new(p, code, m)
}
fn one_int(t: &[Token], i: usize) -> bool {
    matches!(t.get(i).map(|v| &v.tok), Some(Tok::Int(Some(_))))
}
fn arms(t: &[Token], at: usize, value: &str) -> R<(usize, Vec<Token>)> {
    if !word(t, at, "match") || !word(t, at+1, value) || !word(t, at+2, "over") {
        return Err(fail(t[at].pos, "E_UNSUPPORTED", "specialized Any requires a simple match value over"));
    }
    let mut i = at+3;
    let mut output = t[at..at+3].to_vec();
    let mut any = false;
    let mut terminal = false;
    let mut count = 0;
    while i < t.len() && !word(t,i,"end") {
        if !word(t,i,"on") {
            return Err(fail(t[i].pos, "E_SYNTAX", "expected 'on' in static Any match"));
        }
        if count >= 32 {return Err(fail(t[i].pos,"E_SOURCE_LIMIT","too many Any match branches"));}
        count+=1;
        let start=i;
        let (matches_type,pattern_end) = if word(t,i+1,"is") {
            let supported = ["int","string","bool"];
            let ty = supported.iter().find(|ty| word(t,i+2,ty))
                .ok_or_else(|| fail(t[i+1].pos,"E_UNSUPPORTED_TYPE","unsupported Any type pattern"))?;
            (*ty=="int",i+3)
        } else if word(t,i+1,"_") {
            (true,i+2)
        } else {
            return Err(fail(t[i+1].pos,"E_UNSUPPORTED","Any specialization admits only type-test and wildcard arms"));
        };
        let mut arrow=pattern_end;
        let mut p=0usize;
        while arrow<t.len() && t[arrow].tok!=Tok::Arrow {
            if !word(t,pattern_end,"when") {
                return Err(fail(t[arrow].pos,"E_SYNTAX","Any arm requires ->"));
            }
            // Guards may not include nested braced closures in this subset.
            if matches!(t[arrow].tok,Tok::LBrace|Tok::RBrace|Tok::Eof) {
                return Err(fail(t[arrow].pos,"E_UNSUPPORTED","unbounded Any guard"));
            }
            arrow+=1;p+=1;
            if p>80 {return Err(fail(t[pattern_end].pos,"E_SOURCE_LIMIT","Any guard budget"));}
        }
        if arrow>=t.len() || t.get(arrow+1).map(|v|&v.tok)!=Some(&Tok::LBrace){
            return Err(fail(t[start].pos,"E_SYNTAX","Any type arm requires braced body"));
        }
        let mut close=arrow+2;
        let mut depth=1usize;
        while close<t.len() && depth>0 {
            match t[close].tok {
                Tok::LBrace => depth+=1,
                Tok::RBrace => depth-=1,
                Tok::Eof => break,
                _=>{}
            }
            if depth>32{return Err(fail(t[close].pos,"E_NESTING","Any arm nesting budget"));}
            close+=1;
        }
        if depth!=0 {return Err(fail(t[start].pos,"E_EOF","unclosed Any match arm"));}
        if matches_type && !terminal {
            any=true;
            output.push(t[start].clone()); // on
            output.push(Token{tok:Tok::Ident("_".into()),pos:t[start+1].pos});
            output.extend_from_slice(&t[pattern_end..close]);
            if !word(t,pattern_end,"when"){terminal=true;}
        }
        i=close;
    }
    if i==t.len(){return Err(fail(t[at].pos,"E_EOF","unclosed Any match"));}
    if !any || !terminal {
        return Err(fail(t[at].pos,"E_UNSUPPORTED","static Any match must cover its concrete type with an unguarded arm"));
    }
    output.push(t[i].clone()); // end
    Ok((i+1,output))
}

/// Retain literal payload values, but eliminate an unnecessary type tag only
/// when every source-visible path to the admitted Any is statically int.
pub fn lower(mut t: Vec<Token>) -> R<Vec<Token>> {
    if !t.iter().any(|x| matches!(&x.tok,Tok::Ident(n) if n=="Any" || n=="is")) {
        return Ok(t);
    }
    let any_positions:Vec<usize>=(0..t.len()).filter(|&i|word(&t,i,"Any")).collect();
    if any_positions.is_empty(){return Ok(t);}
    if any_positions.len()!=2 {
        return Err(fail(t[any_positions[0]].pos,"E_UNSUPPORTED_TYPE",
            "the checked Any profile needs exactly one concrete value and one monomorphic function"));
    }
    let funcs:(usize,usize,usize,String,String)={
        let mut selected=None;
        for i in 0..t.len().saturating_sub(5) {
            if word(&t,i,"fnc") && matches!(t.get(i+1).map(|x|&x.tok),Some(Tok::Ident(_)))
                && t[i+2].tok==Tok::LParen && word(&t,i+3,"Any")
                && matches!(t.get(i+4).map(|x|&x.tok),Some(Tok::Ident(_)))
                && t[i+5].tok==Tok::RParen {
                let Tok::Ident(name)=&t[i+1].tok else {unreachable!()};
                let Tok::Ident(param)=&t[i+4].tok else {unreachable!()};
                if selected.is_some(){return Err(fail(t[i].pos,"E_UNSUPPORTED","multiple Any functions need dynamic dispatch"));}
                selected=Some((i,i+3,i+4,name.clone(),param.clone()));
            }
        }
        selected.ok_or_else(||fail(t[any_positions[0]].pos,"E_UNSUPPORTED_TYPE",
            "Any requires a single nonescaping, one-param function"))?
    };
    if funcs.0>0 && word(&t,funcs.0-1,"pub"){
        return Err(fail(t[funcs.0].pos,"E_UNSUPPORTED","public Any callables require a real dynamic ABI"));
    }
    let binding=(0..t.len().saturating_sub(5)).find(|&i|word(&t,i,"val")
        && word(&t,i+1,&funcs.4) && t[i+2].tok==Tok::Colon
        && word(&t,i+3,"Any") && t[i+4].tok==Tok::Assign && one_int(&t,i+5));
    let bi=binding.ok_or_else(||fail(t[any_positions[0]].pos,"E_UNSUPPORTED_TYPE",
        "Any value must be immutable and initialized with a checked int literal"))?;
    if !any_positions.contains(&(funcs.1)) || !any_positions.contains(&(bi+3)) {
        return Err(fail(t[any_positions[0]].pos,"E_UNSUPPORTED_TYPE",
            "unexpected Any type occurrence outside monomorphic declaration"));
    }
    // Require a single immutable literal initializer with no hidden call,
    // constructor, generic or alias. The typed checker still validates the
    // actual i64 range and all subsequent arithmetic on the original value.
    if t.get(bi+6).map(|x|&x.tok)!=Some(&Tok::Semi){
        return Err(fail(t[bi].pos,"E_UNSUPPORTED_TYPE","Any initializer must be one scalar literal"));
    }
    // Every call to the specialized function must pass this proven binding,
    // and its function symbol cannot escape or be invoked through a table.
    for i in 0..t.len() {
        if !word(&t,i,&funcs.3) || i==funcs.0+1 {continue;}
        if t.get(i+1).map(|x|&x.tok)!=Some(&Tok::LParen) || !word(&t,i+2,&funcs.4)
            || t.get(i+3).map(|x|&x.tok)!=Some(&Tok::RParen)
            || (i>0 && t[i-1].tok==Tok::Dot) {
            return Err(fail(t[i].pos,"E_UNSUPPORTED","Any callable must be invoked directly with the proven concrete value"));
        }
    }
    // Let the ordinary checker continue to reject direct type mismatches.
    for &i in &any_positions {t[i].tok=Tok::Ident("int".into());}
    let mut result=Vec::with_capacity(t.len());
    let mut i=0;
    let mut done=false;
    while i<t.len() {
        if word(&t,i,"match") && word(&t,i+1,&funcs.4) && word(&t,i+2,"over") {
            if done {return Err(fail(t[i].pos,"E_UNSUPPORTED","only one monomorphic Any match")); }
            let (end,rewritten)=arms(&t,i,&funcs.4)?;
            result.extend(rewritten);i=end;done=true;continue;
        }
        // 'value is int' becomes true only because the *only* admitted
        // value is statically proven to be int. Preserve the remainder of
        // the runtime condition (e.g. value == 42) without constant folding.
        if word(&t,i,&funcs.4) && word(&t,i+1,"is") {
            if !matches!(t.get(i+2).map(|x|&x.tok),Some(Tok::Ident(_))) {
                return Err(fail(t[i].pos,"E_SYNTAX","type test needs nominal type"));
            }
            if !["int","string","bool"].iter().any(|n|word(&t,i+2,n)) {
                return Err(fail(t[i+2].pos,"E_UNSUPPORTED_TYPE","unsupported static Any type test"));
            }
            let matched=word(&t,i+2,"int");
            result.push(Token{tok:Tok::Ident(if matched {"true".into()} else {"false".into()}),pos:t[i].pos});
            i+=3;continue;
        }
        result.push(t[i].clone());i+=1;
    }
    if !done {return Err(fail(t[funcs.0].pos,"E_UNSUPPORTED","monomorphic Any requires a matched type-test arm"));}
    if result.len()>crate::lexer::MAX_TOKENS {
        return Err(fail(Pos{line:1,col:1},"E_SOURCE_LIMIT","specialized Any source exceeds token budget"));
    }
    Ok(result)
}

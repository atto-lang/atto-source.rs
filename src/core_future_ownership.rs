//! Compiler-core ownership admission for sealed Future-family types.
//! This is a *negative preflight*, not executable Future lowering.
//! It runs before any Wasm production, rejects obvious affine violations,
//! and always ends with E_UNSUPPORTED_TYPE until typed MIR/await exists.
use crate::diag::{Diagnostic,Pos};
use crate::lexer::{Tok,Token};

type R<T> = Result<T,Diagnostic>;
const FUTURES: &[&str] = &["Future","TaskFuture","ActorFuture"];
fn word(t: &[Token],i:usize,s:&str)->bool {
    matches!(t.get(i).map(|x| &x.tok),Some(Tok::Ident(v)) if v==s)
}
fn sealed(t: &[Token],i:usize)->bool {
    matches!(t.get(i).map(|x| &x.tok),Some(Tok::Ident(s)) if FUTURES.contains(&s.as_str()))
}
fn err(p:Pos,code:&'static str,msg:&str)->Diagnostic {
    Diagnostic::new(p,code,msg)
}
fn close(t: &[Token],open:usize,left:Tok,right:Tok)->R<usize> {
    let mut depth=0usize;
    for i in open..t.len() {
        if t[i].tok==left { depth+=1; }
        if t[i].tok==right {
            if depth==0 {return Err(err(t[i].pos,"E_SYNTAX","unbalanced Future signature"));}
            depth-=1;
            if depth==0 {return Ok(i);}
        }
        if depth>128{return Err(err(t[open].pos,"E_NESTING","Future signature is too deeply nested"));}
    }
    Err(err(t[open].pos,"E_EOF","unterminated Future signature"))
}
#[derive(Debug)]
struct Receipt {name:String,owned:bool}
fn receipt(t:&[Token],start:usize,end:usize)->R<Option<Receipt>> {
    let mut found=None;
    for i in start..end {
        if !sealed(t,i) || t.get(i+1).map(|x| &x.tok)!=Some(&Tok::Lt) {continue;}
        if found.is_some(){return Err(err(t[i].pos,"E_UNSUPPORTED_TYPE","multiple sealed Future receipts per parameter are not lowered"));}
        let generic_end=close(t,i+1,Tok::Lt,Tok::Gt)?;
        if generic_end>=end {
            return Err(err(t[i].pos,"E_SYNTAX","Future type escapes its formal parameter"));
        }
        let name=match t.get(generic_end+1).map(|x|&x.tok){
            Some(Tok::Ident(n)) if generic_end+1<end=>n.clone(),
            _=>return Err(err(t[i].pos,"E_SYNTAX","Future receipt requires a named parameter"))
        };
        let owned=(start..i).any(|j|word(t,j,"take")&&t.get(j+1).is_some_and(|x|x.tok==Tok::Bang));
        found=Some(Receipt{name,owned});
    }
    Ok(found)
}
fn check_body(t:&[Token],start:usize,end:usize,receipts:&[Receipt])->R<()> {
    for r in receipts {
        let mut moved=false;
        // Conservative region-wide loan: lexical lifetime/alias proofs require
        // typed MIR. Over-rejecting is allowed only while this path always
        // returns E_UNSUPPORTED_TYPE for otherwise valid Future programs.
        let mut loan_live=false;
        let mut i=start;
        while i<end {
            let is_receipt=|j|word(t,j,&r.name);
            if word(t,i,"borrow") && t.get(i+1).is_some_and(|x|x.tok==Tok::Bang)
                && is_receipt(i+2) {
                if moved {
                    return Err(err(t[i].pos,"E_MOVED","cannot borrow a consumed Future receipt"));
                }
                loan_live=true;
                i+=3;
                continue;
            }
            if word(t,i,"take") && t.get(i+1).is_some_and(|x|x.tok==Tok::Bang)
                && is_receipt(i+2) {
                if !r.owned {
                    return Err(err(t[i].pos,"E_OWNERSHIP","cannot transfer a borrowed Future formal"));
                }
                if moved {
                    return Err(err(t[i].pos,"E_MOVED","Future receipt has already been moved"));
                }
                if loan_live {
                    return Err(err(t[i].pos,"E_BORROW","cannot transfer Future while a loan is live"));
                }
                moved=true;
                i+=3;
                continue;
            }
            if word(t,i,"await") && is_receipt(i+1) {
                if !r.owned {
                    return Err(err(t[i].pos,"E_OWNERSHIP",
                        "await on a sealed Future requires a take! formal parameter"));
                }
                if moved {
                    return Err(err(t[i].pos,"E_MOVED",
                        "sealed Future receipt is affine and cannot be awaited twice or after take!"));
                }
                if loan_live {
                    return Err(err(t[i].pos,"E_BORROW",
                        "Future receipt cannot be awaited while a loan is live"));
                }
                moved=true;
                i+=2;
                continue;
            }
            if moved && is_receipt(i) {
                return Err(err(t[i].pos,"E_MOVED","Future receipt used after await/take!"));
            }
            i+=1;
        }
    }
    Ok(())
}
/// Inspect only tokenized, bounded signature and await sites; strings/comments
/// are excluded by the production lexer. Even valid syntax remains unsupported.
/// Do not use this gate as proof of alias/liveness, suspension or actor runtime.
pub fn preflight(t:&[Token])->R<()> {
    let mut seen=false;
    // atto.task.v1 admits exactly `Future<int>` task receipts in programs with
    // susp callables; the checker enforces their affine await-once use.
    let mut only_task_futures=true;
    let has_susp=(0..t.len()).any(|i|(word(t,i,"susp")||word(t,i,"suspends"))&&word(t,i+1,"fnc"));
    let mut any_receipt=false;
    for i in 0..t.len() {
        if sealed(t,i) && t.get(i+1).is_some_and(|x|x.tok==Tok::Lt) {
            seen=true;
            if !(word(t,i,"Future") && word(t,i+2,"int") && t.get(i+3).is_some_and(|x|x.tok==Tok::Gt)) {
                only_task_futures=false;
            }
        }
        if (word(t,i,"new")&&sealed(t,i+1)) ||
           ((word(t,i,"class")||word(t,i,"actor")||word(t,i,"struct"))
              &&sealed(t,i+1)) {
            return Err(err(t[i].pos,"E_RESERVED",
                "sealed Future-family types belong to compiler core, not guest declarations"));
        }
    }
    if !seen {return Ok(());}
    let mut i=0;
    while i<t.len() {
        if !word(t,i,"fnc") {i+=1;continue;}
        let Some(name)=t.get(i+1) else {break;};
        if !matches!(name.tok,Tok::Ident(_)){i+=1;continue;}
        // Function generics precede '('; do not mistake nested return generics
        // or executable code for a formal-parameter declaration.
        let mut at=i+2;
        if t.get(at).is_some_and(|x|x.tok==Tok::Lt) {
            at=close(t,at,Tok::Lt,Tok::Gt)?+1;
        }
        if t.get(at).is_none_or(|x|x.tok!=Tok::LParen){i+=1;continue;}
        let param_end=close(t,at,Tok::LParen,Tok::RParen)?;
        let mut receipts=Vec::<Receipt>::new();
        let mut from=at+1;
        let mut nested_angle=0usize;
        for j in at+1..=param_end {
            if j!=param_end {
                match t[j].tok {
                    Tok::Lt=>nested_angle+=1,
                    Tok::Gt=>nested_angle=nested_angle.saturating_sub(1),
                    _=>{}
                }
            }
            if j==param_end || (t[j].tok==Tok::Comma&&nested_angle==0) {
                if let Some(r)=receipt(t,from,j)?{receipts.push(r);any_receipt=true;}
                from=j+1;
            }
        }
        let mut body=param_end+1;
        while body<t.len() &&
          t[body].tok!=Tok::LBrace && t[body].tok!=Tok::Semi &&
          t[body].tok!=Tok::Eof {body+=1;}
        if body<t.len() && t[body].tok==Tok::LBrace {
            let end=close(t,body,Tok::LBrace,Tok::RBrace)?;
            check_body(t,body+1,end,&receipts)?;
            i=end+1;
        } else {i=body.saturating_add(1);}
    }
    if has_susp && only_task_futures && !any_receipt {
        return Ok(());
    }
    Err(err(t.first().map_or(Pos::default(),|x|x.pos),"E_UNSUPPORTED_TYPE",
        "sealed Future/TaskFuture/ActorFuture awaits need typed affine MIR and a trusted asynchronous runtime; no guest code was emitted"))
}

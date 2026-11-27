//! Narrow automatic prelude pilot; full module/linker integration is separate.
//! Exact source from common-source #164, SHA-256 a411966eb3188104cd13cc2201b4d1d0de1e12404378aba313b396ae726a3376.
use crate::{Diagnostic, Pos, lexer::{self, Tok, Token}, verified_quantities, wasm};

const CATALOG: &str = include_str!("../stdlib/core/measure.atto");
fn known(prefix: &str, name: &str) -> bool {
    CATALOG.lines().any(|line| line.strip_prefix(prefix).is_some_and(|rest| {
        rest.split(|ch: char| !ch.is_ascii_alphanumeric() && ch != '_').next() == Some(name)
    }))
}

pub(crate) fn requested(source: &str, tokens: &[Token]) -> bool {
    let ident = |t: &Token, expected: &str| matches!(&t.tok, Tok::Ident(n) if n == expected);
    // Do not seize general modules, user declarations, imports, methods or
    // ordinary comparisons. Only the file-leading public scalar main pilot.
    if !matches!(tokens.get(..4), Some([a,b,c,d]) if ident(a,"pub") && ident(b,"fnc") && ident(c,"main") && d.tok==Tok::LParen) {
        return false;
    }
    let lines: Vec<_> = source.lines().collect();
    for w in tokens.windows(4) {
        if matches!(w[0].tok,Tok::Int(_)) && w[1].tok==Tok::Lt && w[3].tok==Tok::Gt {
            if let Tok::Ident(name)=&w[2].tok {
                let a=w[0].pos; let b=w[1].pos;
                if known("define unit ",name) && a.line==b.line && b.col>a.col &&
                    lines.get(a.line.saturating_sub(1) as usize).is_some_and(|line| {
                        let digits:String=line.chars().skip(a.col.saturating_sub(1) as usize).take((b.col-a.col) as usize).collect();
                        !digits.is_empty() && digits.chars().all(|ch|ch.is_ascii_digit()||ch=='_')
                    }) { return true; }
            }
        }
        if ident(&w[0],"val") {
            if let (Tok::Ident(name),Tok::Ident(_))=(&w[1].tok,&w[2].tok) {
                if w[3].tok==Tok::Assign && known("type ",name) { return true; }
            }
            if w[2].tok==Tok::Colon {
                if let Tok::Ident(name)=&w[3].tok {
                    if known("type ",name) { return true; }
                }
            }
        }
    }
    false
}
pub(crate) fn compile(source: &str) -> Result<Vec<u8>,Diagnostic> {
    // The catalog is trusted compiler metadata, not user-controlled input.
    // It is kept outside the user's source byte cap, but remains bounded.
    if source.len()>crate::SOURCE_CAP {
        return Err(Diagnostic::new(Pos{line:1,col:1},"E_SOURCE_LIMIT","user source exceeds input cap"));
    }
    let prefix=format!("{CATALOG}\n");
    let offset=prefix.bytes().filter(|b|*b==b'\n').count() as u32;
    let expanded=format!("{prefix}{source}");
    let remap=|mut d:Diagnostic| {
        if d.pos.line>offset {d.pos.line-=offset;}
        else {d.pos=Pos{line:1,col:1};}
        d
    };
    let tokens=lexer::lex(&expanded).map_err(remap)?;
    match verified_quantities::lower(&expanded,&tokens).map_err(remap)? {
        Some(verified_quantities::Lowered::Typed(program))=>{
            let bytes=wasm::emit_quantity_program(&program);
            if bytes.len()>wasm::MAX_OUTPUT {return Err(Diagnostic::new(Pos{line:1,col:1},"E_OUTPUT_LIMIT","generated module exceeds 1 MiB"));}
            Ok(bytes)
        }
        _=>Err(Diagnostic::new(Pos{line:1,col:1},"E_VERIFY_UNSUPPORTED","measurement prelude requires the bounded typed profile"))
    }
}

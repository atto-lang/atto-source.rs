//! Bounded source-derived diamond CFG, not full typed MIR or executable guest.
//! One literal-boolean if/else inside a critical callback is supported.
//! The production target parser validates original source; each branch must
//! pass the strict V0 concrete-field/effect checker. Both paths are checked.
use crate::actor_state_mir_v0::{derive_bounded_guard_mir, FieldEvent, FieldEventKind};
use crate::diag::{Diagnostic, Pos};
use crate::lexer::{lex, Tok, Token};
use crate::target_syntax::parse_target_declarations;
const MAX_SOURCE_BYTES: usize = 65536;
const MAX_EVENTS: usize = 512;
#[derive(Clone, Debug)]
pub struct BoundedDiamondCfg {
    pub source_sha256: String,
    pub field_count: usize,
    pub exclusive: bool,
    pub condition: bool,
    pub then_events: Vec<FieldEvent>,
    pub else_events: Vec<FieldEvent>,
}
fn err(t: &[Token], i: usize, what: &'static str) -> Diagnostic {
    Diagnostic::new(t.get(i).map(|x| x.pos).unwrap_or_default(),
        "E_CFG_UNVERIFIED", what)
}
fn word(t: &[Token], i: usize, v: &str) -> bool {
    matches!(t.get(i).map(|x| &x.tok), Some(Tok::Ident(s)) if s == v)
}
fn offset(s: &str, p: Pos) -> Option<usize> {
    if p.line == 0 || p.col == 0 { return None }
    let (mut line, mut col) = (1, 1);
    for (index, c) in s.char_indices() {
        if (line, col) == (p.line, p.col) { return Some(index); }
        if c == '\n' { line += 1; col = 1 } else { col += 1 }
    }
    if (line, col) == (p.line, p.col) { Some(s.len()) } else { None }
}
fn mask(buf: &mut [u8], a: usize, b: usize) -> bool {
    if a > b || b > buf.len() { return false; }
    for byte in &mut buf[a..b] {
        if *byte != b'\n' && *byte != b'\r' { *byte = b' '; }
    }
    true
}
pub fn derive_bounded_diamond_cfg(src: &str) -> Result<BoundedDiamondCfg, Diagnostic> {
    if src.len() > MAX_SOURCE_BYTES {
        return Err(Diagnostic::new(Pos::default(), "E_SOURCE_LIMIT",
            "diamond source exceeds bounded input"));
    }
    let tokens = lex(src)?;
    let headers = parse_target_declarations(&tokens)?;
    if headers.receivers.len()!=1 || headers.receiver_sites.len()!=1 {
        return Err(err(&tokens, 0, "expected one real actor receiver"));
    }
    let site=&headers.receiver_sites[0];
    let starts:Vec<_>=(site.method_start..site.end)
        .filter(|&i| word(&tokens,i,"if")).collect();
    if starts.len()!=1 {
        return Err(err(&tokens,site.method_start,"expected one source if/else"));
    }
    let at=starts[0];
    if at==0 || !matches!(tokens[at-1].tok,Tok::LBrace) ||
        !(word(&tokens,at+1,"true") || word(&tokens,at+1,"false")) ||
        !word(&tokens,at+2,"then") {
        return Err(err(&tokens,at,"condition must be literal and first in guard"));
    }
    let else_ids:Vec<_>=((at+3)..site.end)
        .filter(|&i|word(&tokens,i,"else")).collect();
    let fi_ids:Vec<_>=((at+3)..site.end)
        .filter(|&i|word(&tokens,i,"fi")).collect();
    if else_ids.len()!=1 || fi_ids.len()!=1 ||
       !(at+3<else_ids[0] && else_ids[0]+1<fi_ids[0]) ||
       !matches!(tokens.get(fi_ids[0]+1).map(|x|&x.tok),Some(Tok::RBrace)) ||
       ((at+3)..fi_ids[0]).any(|i|word(&tokens,i,"if") ||
           word(&tokens,i,"then") || word(&tokens,i,"fi")) {
        return Err(err(&tokens,at,"incomplete or nested diamond"));
    }
    let loc=|i:usize|offset(src,tokens[i].pos).ok_or_else(||
        err(&tokens,i,"source/token byte offsets not authenticated"));
    let if_start=loc(at)?;
    let then_end=loc(at+2)?+4;
    let else_start=loc(else_ids[0])?;
    let else_end=else_start+4;
    let fi_start=loc(fi_ids[0])?;
    let fi_end=fi_start+2;
    let mut then_src=src.as_bytes().to_vec();
    let mut else_src=then_src.clone();
    if !mask(&mut then_src,if_start,then_end) ||
       !mask(&mut then_src,else_start,fi_end) ||
       !mask(&mut else_src,if_start,else_end) ||
       !mask(&mut else_src,fi_start,fi_end) {
        return Err(err(&tokens,at,"untrusted conditional source span"));
    }
    let th=String::from_utf8(then_src)
        .map_err(|_|err(&tokens,at,"invalid source UTF8 masking"))?;
    let el=String::from_utf8(else_src)
        .map_err(|_|err(&tokens,at,"invalid source UTF8 masking"))?;
    // V0 proves *each entire masked source unit* is exactly one supported
    // actor/method/guard, including field initializers and every arm event.
    let a=derive_bounded_guard_mir(&th)?;
    let b=derive_bounded_guard_mir(&el)?;
    if a.actor!=b.actor || a.method!=b.method ||
       a.field_count!=b.field_count || a.exclusive!=b.exclusive ||
       a.events.len().saturating_add(b.events.len())>MAX_EVENTS {
        return Err(err(&tokens,at,"incompatible or oversized diamond effects"));
    }
    Ok(BoundedDiamondCfg {
        source_sha256:crate::sha256::hex_digest(src.as_bytes()),
        field_count:a.field_count,exclusive:a.exclusive,
        condition:word(&tokens,at+1,"true"),
        then_events:a.events,else_events:b.events,
    })
}
impl BoundedDiamondCfg {
    pub fn to_wire(&self)->String {
        let mut s=format!("ATTO_ACTOR_STATE_CFG_V1\t{}\nROOT\t1\t{}\nLOAN\t1\t{}\n",
            self.source_sha256,self.field_count,
            if self.exclusive {"EXCLUSIVE"}else{"SHARED"});
        s.push_str(&format!("BLOCK\t0\nBORROW\t1\nBRANCH\t{}\t1\t2\n",
            if self.condition {"TRUE"}else{"FALSE"}));
        for (block,events) in [(1,&self.then_events),(2,&self.else_events)] {
            s.push_str(&format!("BLOCK\t{block}\n"));
            for e in events {
                s.push_str(&format!("{}\t{}\t{}\t{}\n",
                    if e.kind==FieldEventKind::Read {"READ"} else {"WRITE"},
                    e.field,e.pos.line,e.pos.col));
            }
            s.push_str("END\t1\nGOTO\t3\n");
        }
        s.push_str("BLOCK\t3\nRETURN\nDONE\n");
        s
    }
}

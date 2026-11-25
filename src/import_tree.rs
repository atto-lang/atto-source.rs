//! Bounded static dotted import trees, flattened before binding publication.
use crate::{Diagnostic, Pos};
use crate::import_v2::Kind;
use crate::lexer::{Tok, Token};
use crate::static_import_graph::{Scope, ExportView, Tier};
use std::collections::BTreeSet;
type R<T> = Result<T, Diagnostic>;
const MAX: usize = 256;
const DEPTH: usize = 8;
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Leaf {
    pub path: String,
    pub binding: String,
    pub kind: Option<Kind>,
    pub explicit_alias: bool,
    pub group_self: bool,
}
struct Cursor<'a> { t: &'a [Token], i: usize, out: Vec<Leaf> }
impl Cursor<'_> {
    fn tok(&self) -> &Tok { self.t.get(self.i).map_or(&Tok::Eof, |t| &t.tok) }
    fn err(&self, code: &'static str, msg: &str) -> Diagnostic {
        Diagnostic::new(self.t.get(self.i).map_or(Pos::default(), |t| t.pos), code, msg)
    }
    fn take(&mut self, t: &Tok) -> bool {
        if self.tok()==t { self.i+=1; true } else { false }
    }
    fn word(&mut self,s:&str)->bool { self.take(&Tok::Ident(s.into())) }
    fn name(&mut self, root:bool)->R<String> {
        match self.tok() {
            Tok::Ident(s) if (root && matches!(s.as_str(),"pkg"|"self"|"super")) ||
                (!s.is_empty() && s.len()<=64 && !s.starts_with("__atto_")
                && !crate::parser::is_reserved(s)
                && !matches!(s.as_str(),"pkg"|"self"|"super"|"enum"|"use"|"declare"|"mtd"|"as"|"from")) => {
                let s=s.clone(); self.i+=1; Ok(s)
            }
            _=>Err(self.err("E_SYNTAX","expected static import identifier")),
        }
    }
    fn kind(&mut self)->Option<Kind>{
        let k=match self.tok() {
            Tok::Ident(s)=>match s.as_str(){
                "class"=>Kind::Class,"fnc"=>Kind::Fnc,"module"=>Kind::Module,
                "namespace"=>Kind::Namespace,"const"=>Kind::Const,"actor"=>Kind::Actor,
                "enum"=>Kind::Enum,"trait"=>Kind::Trait,"interface"=>Kind::Interface,
                "type"=>Kind::Type,_=>return None,
            },_=>return None,
        };self.i+=1;Some(k)
    }
    fn tree(&mut self, prefix:&[String], depth:usize)->R<()> {
        if depth>DEPTH { return Err(self.err("E_SOURCE_LIMIT","import tree nesting exceeds eight")); }
        let kind=self.kind();
        let mut parts=prefix.to_vec();
        let group_self=!prefix.is_empty() && self.word("self");
        if !group_self {
            parts.push(self.name(prefix.is_empty())?);
            while self.take(&Tok::Dot) {
                if self.take(&Tok::LBrace) {
                    if kind.is_some() { return Err(self.err("E_SYNTAX","specify kind on each tree leaf")); }
                    loop {
                        self.tree(&parts,depth+1)?;
                        if self.take(&Tok::RBrace){break;}
                        if !self.take(&Tok::Comma){return Err(self.err("E_SYNTAX","expected comma or closing brace"));}
                        if self.take(&Tok::RBrace){break;}
                    }
                    return Ok(())
                }
                let repeated_super=parts.iter().all(|p|p=="super") && matches!(self.tok(),Tok::Ident(s) if s=="super");
                parts.push(self.name(repeated_super)?);
                if parts.len()>DEPTH {return Err(self.err("E_SOURCE_LIMIT","import path exceeds eight components"));}
            }
        }
        if parts.len()>DEPTH || self.out.len()>=MAX {return Err(self.err("E_SOURCE_LIMIT","import tree budget exceeded"));}
        let explicit_alias=self.word("as");
        let binding=if explicit_alias {
            let mut alias=self.name(false)?;
            while self.take(&Tok::Dot) {
                if group_self || alias.split('.').count()>=DEPTH {return Err(self.err("E_SYNTAX","invalid import alias path"));}
                alias.push('.');alias.push_str(&self.name(false)?);
            }alias
        }else if prefix.is_empty() && kind==Some(Kind::Namespace) {parts.join(".")}
        else {parts.last().unwrap().clone()};
        self.out.push(Leaf{path:parts.join("."),binding,kind,explicit_alias,group_self});
        Ok(())
    }
}
/// Parse after import; no source discovery, I/O, or partial binding.
pub fn parse(tokens:&[Token], start:usize)->R<(Vec<Leaf>,usize)>{
    let mut c=Cursor{t:tokens,i:start,out:Vec::new()};
    c.tree(&[],0)?;
    if !c.take(&Tok::Semi){return Err(c.err("E_SYNTAX","expected semicolon after import tree"));}
    let mut seen=BTreeSet::new();
    for leaf in &c.out {
        if !seen.insert(&leaf.binding){return Err(c.err("E_DUPLICATE","duplicate tree binding"));}
    }
    Ok((c.out,c.i))
}
/// Resolve all leaves against the pre-tree scope, then publish atomically.
/// No member existence can disambiguate a source root.
pub fn bind(scope:&mut Scope, leaves:&[Leaf], views:&[&ExportView])->R<()> {
    if leaves.len()>MAX {return Err(Diagnostic::new(Pos::default(),"E_SOURCE_LIMIT","import tree budget exceeded"));}
    if leaves.is_empty() {return Err(Diagnostic::new(Pos::default(),"E_SYNTAX","empty import tree"));}
    let mut staged=scope.clone();
    let mut seen=BTreeSet::new();
    for leaf in leaves {
        if !seen.insert(&leaf.binding){return Err(Diagnostic::new(Pos::default(),"E_DUPLICATE","duplicate tree binding"));}
        let target=scope.resolve_in_views(&leaf.path,views)?;
        if leaf.kind.is_some_and(|kind|kind!=target.kind) ||
            (leaf.group_self && !matches!(target.kind,Kind::Namespace|Kind::Module)) ||
            (leaf.binding.contains('.') && !matches!(target.kind,Kind::Namespace|Kind::Module)) {
            return Err(Diagnostic::new(Pos::default(),"E_IMPORT_KIND","tree leaf kind does not match export"));
        }
        staged.bind_verified(&leaf.binding,if leaf.explicit_alias {Tier::Explicit}else{Tier::Deliberate},target)?;
    }
    *scope=staged;Ok(())
}
/// Resolve contextual roots into the current package's canonical namespace
/// path, without touching the filesystem. A headerless self/super is invalid.
pub fn contextual_path(path:&str,current:Option<&str>)->R<String>{
    let invalid = || Diagnostic::new(Pos::default(),"E_IMPORT_PATH","invalid contextual namespace path");
    let component = |p: &str| !matches!(p,"pkg"|"self"|"super") && crate::static_import_graph::valid_component(p);
    // Validate the source namespace before parent traversal can erase bad components.
    if path.len() > DEPTH * 65 { return Err(invalid()); }
    let mut parts=path.split('.');
    let root=parts.next().unwrap_or("");
    let mut base:Vec<&str>=match root{
        "pkg"=>Vec::new(),
        "self"|"super"=>current.filter(|s|!s.is_empty() && s.len()<=DEPTH*65).ok_or_else(||Diagnostic::new(Pos::default(),"E_IMPORT_PATH","contextual root requires a source namespace"))?.split('.').collect(),
        _=>return Err(Diagnostic::new(Pos::default(),"E_IMPORT_PATH","not a contextual import root")),
    };
    if base.len()>DEPTH || base.iter().any(|p| !component(p)) { return Err(invalid()); }
    let mut tail:Vec<_>=parts.take(DEPTH).collect();
    if tail.is_empty() || tail.len()>=DEPTH { return Err(invalid()); }
    if root=="super" {
        let count=1+tail.iter().take_while(|p|**p=="super").count();
        if count>base.len(){return Err(Diagnostic::new(Pos::default(),"E_IMPORT_PATH","import escapes package root"));}
        base.truncate(base.len()-count);tail.drain(..count-1);
        if tail.is_empty() { return Err(invalid()); }
    }
    base.extend(tail);
    if base.is_empty() || base.len()>DEPTH || base.iter().any(|p| !component(p)){
        return Err(Diagnostic::new(Pos::default(),"E_IMPORT_PATH","invalid contextual namespace path"));
    }
    Ok(base.join("."))
}

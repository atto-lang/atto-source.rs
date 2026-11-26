//! Internal successful isolated-check memo for closed direct-function units.
//! Never authorizes imports or skips whole-program checking on a codegen miss.
//! The caller pins compiler/target and reauthenticates all raw source bytes.
//! No persisted proof; no caller-constructed checked units.
use crate::{ast::{Program,Type},check::{self,TProgram},lexer::{self,Token},Diagnostic,Pos};
use std::{collections::BTreeMap,sync::Arc};
type R<T>=Result<T,Diagnostic>;
#[derive(Default)]
pub(crate) struct Memo { units:BTreeMap<(String,String),Arc<Checked>> }
struct Checked { tokens:Vec<Token>, signature:Option<String> }
#[derive(Clone,Copy,Debug,Default)]
pub(crate) struct Work{
    pub(crate) checked:usize,pub(crate) reused:usize,
    pub(crate) same_signature:usize,pub(crate) changed_signature:usize,
    pub(crate) unknown_signature:usize,
}
pub(crate) struct Pass<'a>{
    prior:&'a Memo,next:Memo,work:Work,
}
pub(crate) fn check_closed(funcs:&[Token])->R<(Program,TProgram)>{
    let mut isolated=funcs[..funcs.len()-1].to_vec();
    isolated.extend(lexer::lex("pub fnc main(): int { return 0; }")?);
    let parsed=crate::parser::Parser::new(isolated).parse_program()?;
    let checked=check::check(&parsed)?;
    if !checked.imports.is_empty(){
        return Err(Diagnostic::new(Pos::default(),"E_UNSUPPORTED","registry dependency cannot request ambient host imports"));
    }
    Ok((parsed,checked))
}
fn same_tokens(a:&[Token],b:&[Token])->bool{
    a.len()==b.len()&&a.iter().zip(b).all(|(x,y)|x.tok==y.tok&&x.pos==y.pos)
}
fn scalar(t:Type)->Option<u8>{
    Some(match t {Type::Int=>1,Type::Bool=>2,Type::Void=>3,Type::Float=>4,Type::Str=>5,_=>return None})
}
fn frame(out:&mut Vec<u8>,s:&str){
    out.extend_from_slice(&(s.len() as u32).to_be_bytes());out.extend_from_slice(s.as_bytes());
}
/// A partial callable-interface fingerprint for reporting, not full ABI
/// authority. Includes named parameter order, mutability, scalar types,
/// declared modifiers/effect bounds and inferred escaping throw/raise.
/// Panics, ownership contracts, nominal types, const values, namespace
/// exports and generic specialization require additional witnesses.
fn signature(parsed:&Program,checked:&TProgram,names:&BTreeMap<String,bool>)->Option<String>{
    if !checked.generators.is_empty()||!parsed.classes.is_empty()||!parsed.actors.is_empty()||!parsed.enums.is_empty(){return None;}
    let effects=check::checked_escape_summary(&checked.funcs);
    let mut out=b"atto.registry.scalar-callable-signature.v1\0".to_vec();
    let public:Vec<_>=names.iter().filter(|(_,p)|**p).collect();
    out.extend_from_slice(&(public.len() as u32).to_be_bytes());
    for (name,_) in public{
        let f=parsed.funcs.iter().find(|f|&f.name==name)?;
        let (i,typed)=checked.funcs.iter().enumerate().find(|(_,f)|&f.name==name)?;
        if f.is_generator||f.params.len()!=typed.params.len()||f.ret!=typed.ret{return None;}
        frame(&mut out,name);out.push(scalar(f.ret)?);out.push(u8::from(f.is_pure));
        out.extend_from_slice(&(f.params.len() as u32).to_be_bytes());
        for (p,t) in f.params.iter().zip(&typed.params){
            if p.ty!=*t{return None;}
            frame(&mut out,&p.name);out.push(scalar(p.ty)?);out.push(u8::from(p.mutable));
        }
        match f.declared_effects{
            None=>out.push(0),
            Some(e)=>out.extend([1,u8::from(e.throws),u8::from(e.raises),u8::from(e.panics)]),
        }
        let (throws,raises)=effects[i];out.extend([u8::from(throws),u8::from(raises)]);
    }
    Some(crate::sha256::hex_digest(&out))
}
impl<'a> Pass<'a>{
    pub(crate) fn new(prior:&'a Memo)->Self{Self{prior,next:Memo::default(),work:Work::default()}}
    pub(crate) fn admit(&mut self,pkg:&str,ns:&str,funcs:&[Token],names:&BTreeMap<String,bool>)->R<()>{
        let key=(pkg.to_owned(),ns.to_owned());
        let old=self.prior.units.get(&key);
        let unit=if let Some(old)=old.filter(|old|same_tokens(&old.tokens,funcs)){
            self.work.reused+=1;Arc::clone(old)
        }else{
            let (parsed,checked)=check_closed(funcs)?;
            let sig=signature(&parsed,&checked,names);
            self.work.checked+=1;
            if let Some(old)=old{
                match (&old.signature,&sig){
                    (Some(a),Some(b)) if a==b=>self.work.same_signature+=1,
                    (Some(_),Some(_))=>self.work.changed_signature+=1,
                    _=>self.work.unknown_signature+=1,
                }
            }
            Arc::new(Checked{tokens:funcs.to_vec(),signature:sig})
        };
        self.next.units.insert(key,unit);
        Ok(())
    }
    pub(crate) fn finish(self)->(Memo,Work){(self.next,self.work)}
}

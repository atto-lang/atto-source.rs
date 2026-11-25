//! Deterministic *planning* metadata for scalar closed Records and named Structs.
//! This is NOT a guest ABI emitter or verification of real heap offsets.
//! 64-bit int/float and 8-bit bool are the only ScalarAbiV1-sized fields;
//! strings, nested objects and open records need target-specific layouts.
use std::collections::BTreeSet;
use crate::collection_types::LiteralType as Ty;
use crate::diag::{Diagnostic,Pos};
type R<T> = Result<T,Diagnostic>;
const MAX_FIELDS: usize = 1024;
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Identity { ClosedRecord, NominalStruct(String) }
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FieldSlot {
    pub name: String,
    pub ty: Ty,
    /// Logical field position, never an executable guest pointer offset.
    pub index: usize,
    /// ScalarAbiV1 *candidate* offset; None for unsized/opaque layouts.
    pub scalar_offset: Option<u32>,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plan {
    pub identity: Identity,
    pub fields: Vec<FieldSlot>,
    /// None until all fields are scalar in the explicitly stated candidate ABI.
    pub scalar_bytes: Option<u32>,
    pub scalar_alignment: Option<u32>,
}
fn error(pos:Pos,code:&'static str,msg:impl Into<String>)->Diagnostic {
    Diagnostic::new(pos,code,msg)
}
fn sized(ty:&Ty)->Option<(u32,u32)> {
    match ty {
        Ty::Int | Ty::Float => Some((8,8)),
        Ty::Bool => Some((1,1)),
        _ => None,
    }
}
fn align(value:u32,alignment:u32,at:Pos)->R<u32> {
    value.checked_add(alignment-1).map(|v|v & !(alignment-1))
        .ok_or_else(||error(at,"E_LAYOUT_OVERFLOW","layout offset overflow"))
}
fn build(identity:Identity,mut fields:Vec<(String,Ty)>,pos:Pos,structural:bool)->R<Plan> {
    if fields.len()>MAX_FIELDS {return Err(error(pos,"E_SOURCE_LIMIT","too many shape fields"));}
    if structural {
        // Field-set identity is independent of *source initializer order*.
        fields.sort_unstable_by(|a,b|a.0.cmp(&b.0));
    }
    let mut names=BTreeSet::new();
    for (name,_) in &fields {
        if !names.insert(name) {
            return Err(error(pos,"E_DUPLICATE",format!("duplicate shape field '{name}'")));
        }
    }
    let entirely_sized=fields.iter().all(|(_,t)|sized(t).is_some());
    let mut offset=0u32;
    let mut max_align=1u32;
    let mut slots=Vec::new();
    for (index,(name,ty)) in fields.into_iter().enumerate() {
        let scalar_offset=if entirely_sized {
            let (size,alignment)=sized(&ty).expect("prechecked scalar");
            max_align=max_align.max(alignment);
            offset=align(offset,alignment,pos)?;
            let start=offset;
            offset=offset.checked_add(size).ok_or_else(||
                error(pos,"E_LAYOUT_OVERFLOW","layout size overflow"))?;
            Some(start)
        } else {None};
        slots.push(FieldSlot {name,ty,index,scalar_offset});
    }
    Ok(Plan {
        identity,
        fields:slots,
        scalar_bytes:if entirely_sized {Some(align(offset,max_align,pos)?)} else {None},
        scalar_alignment:if entirely_sized {Some(max_align)} else {None},
    })
}
/// Canonically sorted structural name/type slots. Open Record is not static.
pub fn record(ty:&Ty,pos:Pos)->R<Plan> {
    match ty {
        Ty::Record(fields)=>build(Identity::ClosedRecord,fields.clone(),pos,true),
        Ty::OpenRecord=>Err(error(pos,"E_LAYOUT_OPEN",
            "an open Record cannot be assigned a fixed layout")),
        _=>Err(error(pos,"E_TYPE","Record layout requires a closed Record type")),
    }
}
/// Declared-order logical slots; nominal identity is the full type name.
pub fn nominal_struct(name:&str,fields:&[(String,Ty)],pos:Pos)->R<Plan> {
    if name.is_empty() {return Err(error(pos,"E_TYPE","nominal Struct requires a type name"));}
    build(Identity::NominalStruct(name.to_string()),fields.to_vec(),pos,false)
}

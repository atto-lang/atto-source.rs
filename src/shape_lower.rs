//! Checked source-local scalar product shapes for the browser Wasm compiler.
//!
//! This lowering uses the existing typed **class field / constructor / guest
//! object** implementation, never compile-time simulated stdout. Struct and
//! record member declarations become real nominal classes with constructor
//! initialization and checked property reads. Two-field Tuple/Pair/Entry are
//! distinct internal nominal products with ordered scalar fields.
//!
//! Scope: direct immutable scalar product construction. It is NOT arbitrary
//! structural subtyping, collections, generic runtime reflection, actor
//! transport or value-copy ABI parity. Dynamic/unknown product shapes reject.
use std::collections::BTreeMap;

use crate::diag::{Diagnostic, Pos};
use crate::lexer::{self, Tok, Token};

type R<T> = Result<T, Diagnostic>;
pub const PREFIX: &str = "__atto_shape_";
const MAX_SHAPES: usize = 64;
const MAX_FIELDS: usize = 8;
const MAX_NAME_BYTES: usize = 64;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind { Struct, Record }
#[derive(Clone)]
struct Field { name: String, ty: String }
#[derive(Clone)]
struct Decl { name: String, kind: Kind, fields: Vec<Field>, pos: Pos }

fn word(tokens: &[Token], i: usize, s: &str) -> bool {
    matches!(tokens.get(i).map(|t| &t.tok), Some(Tok::Ident(w)) if w == s)
}
fn error(pos: Pos, code: &'static str, message: impl Into<String>) -> Diagnostic {
    Diagnostic::new(pos, code, message)
}
fn id(tokens: &[Token], i: usize, what: &str) -> R<String> {
    let t = tokens.get(i).ok_or_else(|| error(Pos {line:1,col:1}, "E_EOF",
        format!("missing {what}")))?;
    match &t.tok {
        Tok::Ident(name) if !name.is_empty() && name.len() <= MAX_NAME_BYTES &&
            !matches!(name.as_str(), "as" | "end" | "define" | "new" | "infer" | "struct" | "record" | "pub") =>
            Ok(name.clone()),
        _ => Err(error(t.pos, "E_SYNTAX", format!("invalid {what}"))),
    }
}
fn scalar(word: &str) -> Option<&'static str> {
    match word {
        "int" => Some("int"),
        "string" | "String" => Some("string"),
        "bool" | "boolean" => Some("bool"),
        "float" | "f64" => Some("float"),
        _ => None,
    }
}
fn default(ty: &str) -> &'static str {
    match ty {
        "int" => "0", "bool" => "false", "string" => r#""""#, "float" => "0.0",
        _ => unreachable!("scalar() checked the field"),
    }
}
fn fragment(src: &str, pos: Pos) -> R<Vec<Token>> {
    let mut out = lexer::lex(src)?;
    if out.last().is_some_and(|x| x.tok == Tok::Eof) { out.pop(); }
    for t in &mut out { t.pos = pos; }
    Ok(out)
}
fn synthesized(klass: &str, fields: &[Field]) -> String {
    let mut src = format!("define class {klass} as\n");
    for f in fields {
        // Lowered product storage is populated by the compiler-generated
        // constructor, not by user code. Make that constructor-write authority
        // explicit: bare class fields now default to immutable `val`.
        // This preserves the existing product ABI and initialization order.
        src.push_str(&format!("pub let {} {} = {};\n", f.ty, f.name, default(&f.ty)));
    }
    src.push_str("constructor(");
    for (i, f) in fields.iter().enumerate() {
        if i > 0 { src.push_str(", "); }
        src.push_str(&format!("{} {}", f.ty, f.name));
    }
    src.push_str(") {\n");
    for f in fields {
        src.push_str(&format!("self.{} = {};\n", f.name, f.name));
    }
    src.push_str("}\nend\n");
    src
}

fn parse_decl(t: &[Token], i: usize, kind: Kind) -> R<(Decl, usize)> {
    let pos = t[i].pos;
    let name = id(t, i+2, "product name")?;
    if !word(t, i+3, "as") {
        return Err(error(pos, "E_SYNTAX", "product declaration requires 'define struct/record Name as'"));
    }
    let mut fields = Vec::new();
    let mut j = i+4;
    while !word(t, j, "end") {
        let current = t.get(j).ok_or_else(|| error(pos, "E_EOF","unterminated product declaration"))?;
        if current.tok == Tok::Eof {
            return Err(error(pos, "E_EOF", "unterminated product declaration"));
        }
        if fields.len() >= MAX_FIELDS {
            return Err(error(current.pos, "E_SOURCE_LIMIT", "product shape has too many fields"));
        }
        let ty = match &current.tok {
            Tok::Ident(w) => scalar(w),
            _ => None,
        }.ok_or_else(|| error(current.pos, "E_UNSUPPORTED_TYPE",
            "product field must be a scalar int/string/bool/float"))?;
        let field = id(t,j+1,"product field")?;
        if t.get(j+2).is_none_or(|x| x.tok != Tok::Semi) {
            return Err(error(current.pos,"E_UNSUPPORTED",
                "product declarations admit only semicolon-terminated scalar fields"));
        }
        if fields.iter().any(|f: &Field| f.name == field) {
            return Err(error(current.pos,"E_DUPLICATE",format!("duplicate field '{field}'")));
        }
        fields.push(Field { name: field,ty:ty.to_owned() });
        j+=3;
    }
    // A zero-field named struct has a real (zero-arity) constructor.
    // The record product profile remains explicitly nonempty.
    if fields.is_empty() && kind == Kind::Record {
        return Err(error(pos, "E_UNSUPPORTED", "empty records require the typed Record profile"));
    }
    Ok((Decl{name,kind,fields,pos}, j+1))
}

fn simple_literal(tok: &Tok) -> bool {
    matches!(tok, Tok::Int(Some(_)) | Tok::Float(_) | Tok::Str(_)) ||
        matches!(tok, Tok::Ident(s) if matches!(s.as_str(),"true"|"false"))
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum KeyedMode { Named, InferNamed, InferAnonymous }
fn keyed(t: &[Token], open: usize, mode: KeyedMode) -> R<(BTreeMap<String, Token>, usize)> {
    if t.get(open).is_none_or(|x| x.tok != Tok::LBrace) {
        return Err(error(t[open.saturating_sub(1)].pos,"E_SYNTAX","product initializer requires '{'"));
    }
    let mut values=BTreeMap::new();
    let mut j=open+1;
    while t.get(j).is_some_and(|x| x.tok != Tok::RBrace) {
        if values.len() >= MAX_FIELDS {
            return Err(error(t[j].pos,"E_SOURCE_LIMIT","too many product initializer fields"));
        }
        if mode != KeyedMode::Named && matches!(&t[j].tok, Tok::Str(_)) {
            return Err(error(t[j].pos,"E_STRUCT_FIELD_NAME",
                "struct fields must use identifier names, not quoted Record keys"));
        }
        let field=id(t,j,"initializer field")?;
        if t.get(j+1).is_none_or(|x| x.tok != Tok::Colon) {
            return Err(error(t[j].pos,"E_SYNTAX","product member initializer uses ':'"));
        }
        let value=t.get(j+2).ok_or_else(|| error(t[j].pos,"E_EOF","missing product value"))?;
        if !simple_literal(&value.tok) {
            if mode == KeyedMode::InferAnonymous
                && ((matches!(&value.tok,Tok::LBracket)
                      && t.get(j+3).is_some_and(|x| x.tok == Tok::RBracket))
                    || matches!(&value.tok,Tok::Ident(name) if name == "None"))
            {
                return Err(error(value.pos,"E_TYPE_INFER_AMBIGUOUS",
                    "anonymous struct field None/[] needs an expected payload or element type"));
            }
            return Err(error(value.pos,"E_UNSUPPORTED",
                "source-local keyed products currently require literal scalar fields"));
        }
        if values.insert(field.clone(),value.clone()).is_some() {
            return Err(error(t[j].pos,"E_DUPLICATE",format!("duplicate initialized field '{field}'")));
        }
        j+=3;
        if t.get(j).is_some_and(|x| x.tok == Tok::Comma) {j+=1;}
        else if t.get(j).is_none_or(|x| x.tok != Tok::RBrace) {
            return Err(error(value.pos,"E_SYNTAX","product fields require commas"));
        }
    }
    if t.get(j).is_none() { return Err(error(t[open].pos,"E_EOF","unterminated product literal")); }
    Ok((values,j+1))
}

fn constructor_tokens(decl: &Decl, values: BTreeMap<String,Token>, pos: Pos) -> R<Vec<Token>> {
    if values.len()!=decl.fields.len() ||
        decl.fields.iter().any(|f|!values.contains_key(&f.name)) {
        return Err(error(pos,"E_SHAPE",
            format!("'{}' constructor fields differ from its declared product shape",decl.name)));
    }
    let mut out=fragment(&format!("new {}(",decl.name),pos)?;
    for (i,f) in decl.fields.iter().enumerate() {
        if i>0 {out.push(Token{tok:Tok::Comma,pos});}
        out.push(values[&f.name].clone());
    }
    out.push(Token{tok:Tok::RParen,pos});
    Ok(out)
}
fn synthetic_name(kind: &str, a: &str, b: &str) -> String {
    format!("{PREFIX}{kind}_{a}_{b}")
}

/// Only a binding's explicit nominal annotation supplies a struct type in
/// this bounded lowering profile. Never guess one from similarly named fields.
/// Full contextual argument/return inference belongs in the typed AST pass.
fn binding_keyword(t: &[Token], i: usize) -> bool {
    word(t, i, "val") || word(t, i, "const") || word(t, i, "let")
}
fn token_name(t: &[Token], i: usize) -> Option<&str> {
    match t.get(i).map(|x| &x.tok) {
        Some(Tok::Ident(s)) => Some(s.as_str()),
        _ => None,
    }
}
fn expected_struct_name(t: &[Token], at: usize) -> Option<&str> {
    if at < 4 || t.get(at - 1).is_none_or(|x| x.tok != Tok::Assign) {
        return None;
    }
    // val x: T = infer struct {...}
    if at >= 5 && binding_keyword(t, at - 5)
        && t.get(at - 3).is_some_and(|x| x.tok == Tok::Colon)
        && token_name(t, at - 4).is_some()
    {
        return token_name(t, at - 2);
    }
    // const T x = infer struct {...}
    if binding_keyword(t, at - 4) && token_name(t, at - 2).is_some() {
        return token_name(t, at - 3);
    }
    None
}
fn literal_type(tok: &Tok) -> Option<&'static str> {
    match tok {
        Tok::Int(Some(_)) => Some("int"),
        Tok::Float(_) => Some("float"),
        Tok::Str(_) => Some("string"),
        Tok::Ident(s) if s == "true" || s == "false" => Some("bool"),
        _ => None,
    }
}
/// The narrow guest-AOT profile admits only literal scalar initializers.
/// Collect in source order, not BTreeMap's sorted field-name order.
fn anonymous_fields(t: &[Token], start: usize, end: usize) -> R<Vec<Field>> {
    let mut fields = Vec::new();
    let mut j = start;
    while j < end {
        let name = id(t, j, "anonymous struct field")?;
        let value = t.get(j + 2).ok_or_else(|| error(t[j].pos, "E_EOF",
            "missing anonymous struct field value"))?;
        let ty = literal_type(&value.tok).ok_or_else(|| error(value.pos,
            "E_TYPE_INFER_AMBIGUOUS", "anonymous struct field needs a uniquely known static type"))?;
        fields.push(Field { name, ty: ty.to_owned() });
        j += 3;
        if j < end && t[j].tok == Tok::Comma { j += 1; }
    }
    Ok(fields)
}
fn product_types(t: &[Token], at: usize, pair: bool, pos: Pos) -> R<(String,String,usize)> {
    if t.get(at).is_none_or(|x| x.tok != Tok::Lt) {
        return Err(error(pos,"E_UNSUPPORTED_TYPE","expected product type arguments"));
    }
    let first=token_name(t,at+1).and_then(scalar).ok_or_else(||
        error(pos,"E_UNSUPPORTED_TYPE","product type must be scalar in this profile"))?;
    if pair && t.get(at+2).is_some_and(|x| x.tok == Tok::Gt) {
        return Ok(("string".into(),first.into(),at+3));
    }
    if t.get(at+2).is_none_or(|x| x.tok != Tok::Comma) ||
        t.get(at+4).is_none_or(|x| x.tok != Tok::Gt) {
        return Err(error(pos,"E_UNSUPPORTED_TYPE","expected two scalar product type arguments"));
    }
    let second=token_name(t,at+3).and_then(scalar).ok_or_else(||
        error(pos,"E_UNSUPPORTED_TYPE","second product type must be scalar"))?;
    if pair && first != "string" {
        return Err(error(pos,"E_TYPE","Pair first type argument must be string"));
    }
    Ok((first.into(),second.into(),at+5))
}
fn generics(t: &[Token], index: usize) -> R<(String,String,usize)> {
    product_types(t,index+1,word(t,index,"Pair"),t[index].pos)
}
fn closing(t:&[Token],open:usize) -> R<usize> {
    let pos=t.get(open).or_else(||t.last()).map(|x|x.pos).unwrap_or_default();
    if t.get(open).is_none_or(|x| x.tok != Tok::LParen) {
        return Err(error(pos,"E_SYNTAX","expected '(' for product arguments"));
    }
    let mut expected=Vec::<Tok>::new();
    for i in open..t.len() {
        match &t[i].tok {
            Tok::LParen=>expected.push(Tok::RParen),
            Tok::LBrace=>expected.push(Tok::RBrace),
            Tok::LBracket=>expected.push(Tok::RBracket),
            Tok::RParen|Tok::RBrace|Tok::RBracket=>{
                if expected.pop().as_ref()!=Some(&t[i].tok) {
                    return Err(error(t[i].pos,"E_SYNTAX","mismatched product argument delimiter"));
                }
                if expected.is_empty() {return Ok(i);}
            }
            Tok::Eof=>break,
            _=>{}
        }
        if expected.len()>128 {return Err(error(pos,"E_NESTING","nested product argument limit"));}
    }
    Err(error(pos,"E_EOF","unterminated positional product literal"))
}
fn typed_product(t:&[Token],i:usize,defs:&mut BTreeMap<String,Vec<Field>>)
    -> R<Option<(Vec<Token>,usize)>>
{
    if !word(t,i,"val") ||
        !matches!(t.get(i+1).map(|x|&x.tok),Some(Tok::Ident(_))) ||
        t.get(i+2).is_none_or(|x| x.tok != Tok::Colon) {
        return Ok(None);
    }
    let kind=match t.get(i+3).map(|x|&x.tok) {
        Some(Tok::Ident(s)) if matches!(s.as_str(),"Tuple"|"Pair"|"Entry")=>s.as_str(),
        _=>return Ok(None)
    };
    let (a,b,after)=generics(t,i+3)?;
    if t.get(after).is_none_or(|x| x.tok != Tok::Assign) {
        return Err(error(t[i].pos,"E_SYNTAX","typed product binding requires '='"));
    }
    let open=if kind=="Tuple" {
        after+1
    } else {
        if !word(t,after+1,kind) {
            return Err(error(t[i].pos,"E_TYPE","typed product constructor kind must match the annotation"));
        }
        if t.get(after+2).is_some_and(|x| x.tok == Tok::Dot) && word(t,after+3,"of") {
            let witness=after+4;
            if t.get(witness).is_some_and(|x|x.tok==Tok::Lt) {
                let (wa,wb,end)=product_types(t,witness,kind=="Pair",t[witness].pos)?;
                if wa!=a || wb!=b {
                    return Err(error(t[witness].pos,"E_TYPE","factory type arguments disagree with product annotation"));
                }
                end
            } else { witness }
        } else { after+2 }
    };
    let close=closing(t,open)?;
    if t.get(close+1).is_none_or(|x| x.tok != Tok::Semi) {
        return Err(error(t[close].pos,"E_SYNTAX","product value must end in a semicolon"));
    }
    let cls=synthetic_name(&kind.to_lowercase(),&a,&b);
    if defs.len()>=MAX_SHAPES && !defs.contains_key(&cls) {
        return Err(error(t[i].pos,"E_SOURCE_LIMIT","too many generic product specializations"));
    }
    let named=match kind {
        "Tuple"=>["first","second"],"Pair"=>["first","second"],_=>["key","value"]
    };
    defs.entry(cls.clone()).or_insert_with(||vec![
        Field{name:named[0].into(),ty:a.clone()},Field{name:named[1].into(),ty:b.clone()}
    ]);
    let mut out=t[i..i+3].to_vec();
    out.push(Token{tok:Tok::Ident(cls.clone()),pos:t[i+3].pos});
    out.push(t[after].clone()); // =
    out.push(Token{tok:Tok::Ident("new".into()),pos:t[open].pos});
    out.push(Token{tok:Tok::Ident(cls),pos:t[open].pos});
    out.extend_from_slice(&t[open..=close]);
    out.push(t[close+1].clone()); // ;
    Ok(Some((out,close+2)))
}

// This pre-parser is not a general name resolver. Only infer unqualified
// source-local Pair.of in a program without competing Pair declarations,
// aliases, parameters or qualified access. Unknown forms stay unadmitted.
fn product_factory(t: &[Token], i: usize, kind: &str) -> bool {
    word(t,i,kind) && t.get(i+1).is_some_and(|x| x.tok == Tok::Dot)
        && word(t,i+2,"of")
}
fn product_factory_guard(t: &[Token]) -> R<()> {
    for kind in ["Pair","Entry"] {
        if !t.iter().enumerate().any(|(i,_)| product_factory(t,i,kind)) { continue; }
        for (i, token) in t.iter().enumerate() {
            if !word(t,i,kind) { continue; }
            let prior = i.checked_sub(1).and_then(|p|t.get(p)).map(|x|&x.tok);
            let factory = product_factory(t,i,kind) && !matches!(prior,Some(Tok::Dot | Tok::ColonColon));
            let annotation = matches!(prior,Some(Tok::Colon))
                && t.get(i+1).is_some_and(|x|x.tok==Tok::Lt);
            // Bare legacy constructor is allowed only after the assignment
            // of a matching typed binding. The typed path checks its identity.
            let legacy = matches!(prior,Some(Tok::Assign))
                && t.get(i+1).is_some_and(|x|x.tok==Tok::LParen)
                && (i.saturating_sub(10)..i).any(|start|
                    word(t,start,"val") && token_name(t,start+1).is_some()
                    && t.get(start+2).is_some_and(|x|x.tok==Tok::Colon)
                    && word(t,start+3,kind)
                    && generics(t,start+3).is_ok_and(|(_,_,end)|end+1==i));
            if !factory && !annotation && !legacy {
                return Err(error(token.pos,"E_UNSUPPORTED",
                    format!("{kind}.of requires an unambiguous source-local builtin; competing reference")));
            }
        }
    }
    Ok(())
}
fn inferred_product(t: &[Token], i: usize, defs: &mut BTreeMap<String,Vec<Field>>)
    -> R<Option<(Vec<Token>,usize)>>
{
    if !word(t,i,"val") || token_name(t,i+1).is_none()
        || t.get(i+2).is_none_or(|x|x.tok!=Tok::Assign) { return Ok(None); }
    let kind=if product_factory(t,i+3,"Pair") {"Pair"}
        else if product_factory(t,i+3,"Entry") {"Entry"} else {return Ok(None);};
    let witness=i+6;
    let explicit=if t.get(witness).is_some_and(|x|x.tok==Tok::Lt) {
        Some(product_types(t,witness,kind=="Pair",t[witness].pos)?)
    } else {None};
    let open=explicit.as_ref().map_or(witness,|(_,_,end)|*end);
    let close=closing(t,open)?;
    if t.get(close+1).is_none_or(|x|x.tok!=Tok::Semi) {
        return Err(error(t[close].pos,"E_SYNTAX","product binding requires semicolon"));
    }
    let (a,b)=if let Some((a,b,_))=explicit {(a,b)} else {
        let args=&t[open+1..close];
        if args.len()!=3 || args[1].tok!=Tok::Comma {
            return Err(error(t[open].pos,"E_UNSUPPORTED_TYPE",
                "inferred product requires two scalar literals or explicit factory types"));
        }
        let a=literal_type(&args[0].tok).ok_or_else(||error(args[0].pos,"E_UNSUPPORTED_TYPE","inferred key needs a scalar literal"))?;
        if kind=="Pair" && a!="string" {
            return Err(error(args[0].pos,"E_TYPE","Pair first argument must be string"));
        }
        let b=literal_type(&args[2].tok).ok_or_else(||error(args[2].pos,"E_UNSUPPORTED_TYPE","inferred value needs a scalar literal"))?;
        (a.into(),b.into())
    };
    let cls=synthetic_name(&kind.to_lowercase(),&a,&b);
    if defs.len()>=MAX_SHAPES && !defs.contains_key(&cls) {
        return Err(error(t[i].pos,"E_SOURCE_LIMIT","too many generic product specializations"));
    }
    let named=if kind=="Pair" {["first","second"]} else {["key","value"]};
    defs.entry(cls.clone()).or_insert_with(||vec![
        Field{name:named[0].into(),ty:a},Field{name:named[1].into(),ty:b},
    ]);
    let mut out=t[i..i+3].to_vec();
    out.push(Token{tok:Tok::Ident("new".into()),pos:t[open].pos});
    out.push(Token{tok:Tok::Ident(cls),pos:t[open].pos});
    out.extend_from_slice(&t[open..=close+1]);
    Ok(Some((out,close+2)))
}

/// Lower exactly source-local immutable two-field scalar products through
/// checked class constructors, preserving real runtime allocations and field
/// reads. Not executable for arbitrary structural/collection operations.
pub fn lower(t: Vec<Token>) -> R<Vec<Token>> {
    for x in &t {
        if let Tok::Ident(name)=&x.tok {
            if name.starts_with(PREFIX) {
                return Err(error(x.pos,"E_RESERVED","compiler-owned shape class names cannot appear in source"));
            }
        }
    }
    if !t.iter().enumerate().any(|(i,_)|
        word(&t,i,"struct")||word(&t,i,"record")||
        (word(&t,i,"Tuple")||word(&t,i,"Pair")||word(&t,i,"Entry"))) {
        return Ok(t);
    }
    product_factory_guard(&t)?;
    let mut declared=BTreeMap::<String,Decl>::new();
    let mut first=Vec::new();
    let mut i=0;
    while i<t.len() {
        if word(&t,i,"define") &&
            (word(&t,i+1,"struct")||word(&t,i+1,"record")) {
            let kind=if word(&t,i+1,"struct"){Kind::Struct}else{Kind::Record};
            let (decl,next)=parse_decl(&t,i,kind)?;
            if declared.len()>=MAX_SHAPES {
                return Err(error(decl.pos,"E_SOURCE_LIMIT","too many named product definitions"));
            }
            if declared.contains_key(&decl.name) {
                return Err(error(decl.pos,"E_DUPLICATE",format!("duplicate product '{}'",decl.name)));
            }
            first.extend(fragment(&synthesized(&decl.name,&decl.fields),decl.pos)?);
            declared.insert(decl.name.clone(),decl);
            i=next;continue;
        }
        first.push(t[i].clone());
        i+=1;
    }
    let mut synthetic=BTreeMap::<String,Vec<Field>>::new();
    let mut anonymous_counter=0usize;
    let mut out=Vec::new();
    i=0;
    while i<first.len() {
        if let Some((product,next))=inferred_product(&first,i,&mut synthetic)? {
            out.extend(product);i=next;continue;
        }
        if let Some((product,next))=typed_product(&first,i,&mut synthetic)? {
            out.extend(product);i=next;continue;
        }
        if word(&first,i,"infer") && word(&first,i+1,"struct") {
            let expected = expected_struct_name(&first, i);
            let mode=if expected.is_some() { KeyedMode::InferNamed }
                else { KeyedMode::InferAnonymous };
            let (values,next)=keyed(&first,i+2,mode)?;
            if let Some(name) = expected {
                // A known T selects exactly T, never a same-shape neighbor.
                let target = declared.get(name).ok_or_else(||error(first[i].pos,
                    "E_UNKNOWN_TYPE",format!("expected '{name}' is not a declared struct")))?;
                if target.kind != Kind::Struct {
                    return Err(error(first[i].pos,"E_TYPE",
                        format!("expected '{name}' is not a struct")));
                }
                if let Some(field)=values.keys().find(|f|
                    !target.fields.iter().any(|decl| decl.name.as_str()==f.as_str()))
                {
                    return Err(error(first[i].pos,"E_NO_FIELD",
                        format!("struct '{name}' has no field '{field}'")));
                }
                if let Some(field)=target.fields.iter().find(|f|
                    !values.contains_key(&f.name))
                {
                    return Err(error(first[i].pos,"E_STRUCT_MISSING_FIELD",
                        format!("required field '{}.{}' is missing",name,field.name)));
                }
                out.extend(constructor_tokens(target,values,first[i].pos)?);
            } else {
                // No expected T: create a per-site nominal struct, even when
                // one or several declared structs have exactly these fields.
                if declared.len()+synthetic.len()>=MAX_SHAPES {
                    return Err(error(first[i].pos,"E_SOURCE_LIMIT",
                        "too many product shapes for bounded inference"));
                }
                let fields=anonymous_fields(&first,i+3,next-1)?;
                let name=format!("{PREFIX}infer_{anonymous_counter}");
                anonymous_counter+=1;
                if synthetic.insert(name.clone(),fields.clone()).is_some() {
                    return Err(error(first[i].pos,"E_DUPLICATE",
                        "anonymous struct internal identity collision"));
                }
                let anon=Decl{name,kind:Kind::Struct,fields,pos:first[i].pos};
                out.extend(constructor_tokens(&anon,values,first[i].pos)?);
            }
            i=next;continue;
        }
        if let Tok::Ident(name)=&first[i].tok {
            if let Some(decl)=declared.get(name) {
                if i+1<first.len() && first[i+1].tok==Tok::LBrace {
                    let (values,next)=keyed(&first,i+1,KeyedMode::Named)?;
                    out.extend(constructor_tokens(decl,values,first[i].pos)?);
                    i=next;continue;
                }
            }
        }
        out.push(first[i].clone());
        i+=1;
    }
    if !synthetic.is_empty() {
        if out.last().is_some_and(|x|x.tok==Tok::Eof) {out.pop();}
        let pos=t.first().map(|x|x.pos).unwrap_or(Pos{line:1,col:1});
        for (name,fields) in synthetic {
            out.extend(fragment(&synthesized(&name,&fields),pos)?);
        }
        out.push(Token{tok:Tok::Eof,pos});
    }
    if out.len()>lexer::MAX_TOKENS {
        return Err(error(t[0].pos,"E_SOURCE_LIMIT","checked product specialization exceeds token budget"));
    }
    Ok(out)
}

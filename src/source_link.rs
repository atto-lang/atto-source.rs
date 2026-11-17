//! Bounded executable source-link profile for sealed classes.
//!
//! The trusted build caller supplies exact source bytes and unit IDs. Import
//! syntax binds classes to that supplied catalogue; no untrusted Wasm custom
//! section is treated as authentication. Resolved classes are rewritten to
//! compiler-reserved unique names, the complete qualified graph is checked,
//! and the existing nominal/type/ownership compiler checks and emits a SINGLE
//! executable Wasm module. Runtime inheritance and inherited base layouts are
//! still unsupported; method-copy composition is the only admitted ABI.
//!
//! This is intentionally NOT a generic package manager or binary Wasm linker.
use std::collections::BTreeMap;

use crate::diag::{Diagnostic, Pos};
use crate::lexer::{self, Tok, Token};
use crate::nominal_link::{self, LinkClass, NominalId};
use crate::nominal_lower::{ClassVisibility, Hierarchy};
type R<T> = Result<T, Diagnostic>;
const MAX_UNITS: usize = 32;
const MAX_CLASSES: usize = 96;
const PREFIX: &str = "AttoLinkInternalC";

/// Package/module identity is supplied by a trusted build catalogue, never
/// taken from a source-file comment, import alias, or guest metadata.
#[derive(Clone, Debug)]
pub struct SourceUnit<'a> {
    pub package: &'a str,
    pub namespace: &'a str,
    pub module: &'a str,
    pub source: &'a str,
}
#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
struct UnitKey {
    package: String,
    namespace: String,
    module: String,
}
impl UnitKey {
    fn of(unit: &SourceUnit<'_>) -> Self {
        Self {package:unit.package.into(), namespace:unit.namespace.into(),
            module:unit.module.into()}
    }
    fn parse(s: &str, pos: Pos) -> R<Self> {
        let parts: Vec<&str> = s.split('/').collect();
        if parts.len() != 3 || parts.iter().any(|x| !valid(x)) {
            return Err(err(pos, "E_NAMESPACE",
                "class import target must be an exact package/namespace/module identity"));
        }
        Ok(Self { package:parts[0].into(), namespace:parts[1].into(),
            module:parts[2].into() })
    }
}
fn valid(s: &str) -> bool {
    s.len() <= 64 && !s.is_empty() && (s.as_bytes()[0].is_ascii_alphabetic() || s.as_bytes()[0] == b'_') &&
    s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}
fn err(pos: Pos, code: &'static str, why: impl Into<String>) -> Diagnostic {
    Diagnostic::new(pos, code, why)
}
fn word(t: &[Token], i: usize, s: &str) -> bool {
    matches!(t.get(i).map(|x| &x.tok), Some(Tok::Ident(x)) if x == s)
}
fn identifier(t: &[Token], i: usize) -> R<String> {
    match t.get(i) {
        Some(Token {tok:Tok::Ident(name),pos}) if valid(name) &&
            !matches!(name.as_str(), "as" | "end" | "class" | "define" |
                "pub" | "final" | "sealed" | "non" | "permits" |
                "extends" | "implements" | "from" | "import") =>
            Ok(name.clone()),
        Some(x) => Err(err(x.pos,"E_NAMESPACE","invalid class/import identifier")),
        None => Err(err(Pos {line:1,col:1},"E_EOF","missing class/import identifier")),
    }
}
fn expect(t: &[Token], at: usize, token: Tok, reason: &str) -> R<()> {
    if t.get(at).is_some_and(|x| x.tok == token) { Ok(()) } else {
        Err(err(t.get(at).map_or(Pos {line:1,col:1},|x|x.pos),
            "E_SYNTAX",reason))
    }
}
#[derive(Clone)]
struct Import {
    local: String,
    remote: String,
    unit: UnitKey,
    pos: Pos,
}
#[derive(Clone)]
struct Decl {
    name: String,
    pos: Pos,
    kind: Hierarchy,
    visibility: ClassVisibility,
    parents: Vec<String>,
    permits: Vec<String>,
}
struct Parsed {
    key: UnitKey,
    tokens: Vec<Token>,
    imports: Vec<Import>,
    decls: Vec<Decl>,
    emitted: Vec<Token>,
}
fn list(t: &[Token], index: &mut usize) -> R<Vec<String>> {
    let mut result = Vec::new();
    loop {
        if result.len() >= MAX_CLASSES {
            return Err(err(t.get(*index).map_or(Pos {line:1,col:1},|x|x.pos),
                "E_SOURCE_LIMIT","too many linked class edges"));
        }
        result.push(identifier(t,*index)?);
        *index += 1;
        if t.get(*index).map(|x|&x.tok) != Some(&Tok::Comma) { break; }
        *index += 1;
    }
    Ok(result)
}
fn read_import(t: &[Token], index: &mut usize) -> R<Import> {
    let pos=t[*index].pos;
    if !word(t,*index+1,"class") {
        return Err(err(pos,"E_UNSUPPORTED",
            "source-link profile supports only explicit class imports"));
    }
    let remote=identifier(t,*index+2)?;
    let mut j=*index+3;
    let local=if word(t,j,"as") {
        j+=1;
        let alias=identifier(t,j)?;
        j+=1;
        alias
    } else {remote.clone()};
    if !word(t,j,"from") {
        return Err(err(pos,"E_SYNTAX","class import requires from and a quoted exact source-unit key"));
    }
    j+=1;
    let unit=match t.get(j).map(|x|&x.tok) {
        Some(Tok::Str(source)) => UnitKey::parse(source,pos)?,
        _ => return Err(err(pos,"E_SYNTAX","class import requires a quoted package/namespace/module key")),
    };
    expect(t,j+1,Tok::Semi,"class import must end with ';'")?;
    *index=j+2;
    Ok(Import{local,remote,unit,pos})
}
fn read_export(t: &[Token], index: &mut usize) -> R<(String,ClassVisibility)> {
    let start=*index;
    let pos=t[start].pos;
    if !word(t,start+1,"pub") {
        return Err(err(pos,"E_UNSUPPORTED","source-linked class export must be pub or pub[pkg]"));
    }
    let mut j=start+2;
    let visibility=if t.get(j).map(|x|&x.tok)==Some(&Tok::LBracket) {
        if !word(t,j+1,"pkg") || t.get(j+2).map(|x|&x.tok)!=Some(&Tok::RBracket) {
            return Err(err(pos,"E_SYNTAX","only pub[pkg] is supported"));
        }
        j+=3;
        ClassVisibility::Package
    } else {ClassVisibility::Public};
    if !word(t,j,"class") {
        return Err(err(pos,"E_UNSUPPORTED","source-link profile exports classes only"));
    }
    let name=identifier(t,j+1)?;
    expect(t,j+2,Tok::Semi,"class export requires ';'")?;
    *index=j+3;
    Ok((name,visibility))
}
fn read_class(t: &[Token], start: usize, first: usize) -> R<Option<(Decl,usize)>> {
    let mut j=first;
    if !word(t,j,"define") {return Ok(None)}
    j+=1;
    let public=word(t,j,"pub");
    if public {j+=1;}
    let kind=if word(t,j,"sealed") {j+=1; Hierarchy::Sealed}
        else if word(t,j,"final") {j+=1; Hierarchy::Final}
        else if word(t,j,"non") && t.get(j+1).map(|x|&x.tok)==Some(&Tok::Minus)
            && word(t,j+2,"sealed") {
            if t[j].pos.line!=t[j+1].pos.line || t[j+1].pos.line!=t[j+2].pos.line ||
                t[j+1].pos.col!=t[j].pos.col+3 ||
                t[j+2].pos.col!=t[j+1].pos.col+1 {
                return Err(err(t[j].pos,"E_SEALED","non-sealed must be written contiguously"));
            }
            j+=3; Hierarchy::NonSealed
        } else {Hierarchy::Open};
    if !word(t,j,"class") {return Ok(None)}
    let name=identifier(t,j+1)?;
    j+=2;
    let parents=if word(t,j,"extends") {j+=1; list(t,&mut j)?} else {vec![]};
    if word(t,j,"implements") {
        return Err(err(t[j].pos,"E_UNSUPPORTED",
            "cross-source trait/interface ABI linking is not yet verified"));
    }
    let permits=if word(t,j,"permits") {j+=1; list(t,&mut j)?} else {vec![]};
    if !word(t,j,"as") {
        return Err(err(t.get(j).map_or(t[start].pos,|x|x.pos),
            "E_SYNTAX","expected 'as' in class header"));
    }
    j+=1;
    let mut depth=0usize;
    while j<t.len() {
        if t[j].tok==Tok::LBrace {depth+=1}
        else if t[j].tok==Tok::RBrace {
            if depth==0 {return Err(err(t[j].pos,"E_SYNTAX","orphan class body brace"));}
            depth-=1;
        }
        if depth==0 && word(t,j,"end") {break}
        if t[j].tok==Tok::Eof {break}
        j+=1;
    }
    if j>=t.len() || t[j].tok==Tok::Eof {
        return Err(err(t[start].pos,"E_EOF","unterminated linked class declaration"));
    }
    Ok(Some((Decl {name,pos:t[start].pos,kind,parents,permits,
        visibility:if public {ClassVisibility::Public}else{ClassVisibility::Private}},j+1)))
}
fn parse(unit: &SourceUnit<'_>, entry: bool) -> R<Parsed> {
    let key=UnitKey::of(unit);
    if !valid(unit.package) || !valid(unit.namespace) || !valid(unit.module) {
        return Err(err(Pos {line:1,col:1},"E_NAMESPACE","invalid build-owned source unit identity"));
    }
    if unit.source.len()>crate::SOURCE_CAP {
        return Err(err(Pos {line:1,col:1},"E_SOURCE_LIMIT","linked source unit is too large"));
    }
    let mut tokens=lexer::lex(unit.source)?;
    tokens.pop(); // each unit has one terminal EOF; merge restores just one
    if tokens.iter().any(|t|matches!(&t.tok,Tok::Ident(name)
        if name.starts_with(PREFIX))) {
        return Err(err(Pos {line:1,col:1},"E_RESERVED",
            "compiler-owned linked class symbol must not occur in user source"));
    }
    let mut i=0usize;
    let mut imports=Vec::new();
    while word(&tokens,i,"import") {
        if imports.len()>=MAX_CLASSES {
            return Err(err(tokens[i].pos,"E_SOURCE_LIMIT","too many imported classes"));
        }
        imports.push(read_import(&tokens,&mut i)?);
    }
    let first_source=i;
    let mut decls=Vec::new();
    let mut exports=BTreeMap::new();
    let mut braces=0usize;
    while i<tokens.len() {
        if word(&tokens,i,"import") && braces==0 {
            return Err(err(tokens[i].pos,"E_SYNTAX","linked imports must precede all declarations"));
        }
        if word(&tokens,i,"export") && braces==0 {
            let pos=tokens[i].pos;
            let (name,vis)=read_export(&tokens,&mut i)?;
            if exports.insert(name,vis).is_some() {
                return Err(err(pos,"E_DUPLICATE","duplicate linked class export"));
            }
            continue;
        }
        let start=i;
        let (first, prefix)=if word(&tokens,i,"pub") && word(&tokens,i+1,"define") {
            (i+1,true)
        } else {(i,false)};
        if braces==0 {
            if let Some((mut decl,end))=read_class(&tokens,start,first)? {
                if prefix { decl.visibility=ClassVisibility::Public; }
                if decls.len()>=MAX_CLASSES {
                    return Err(err(tokens[start].pos,"E_SOURCE_LIMIT","too many linked classes"));
                }
                if decls.iter().any(|c:&Decl|c.name==decl.name) {
                    return Err(err(decl.pos,"E_DUPLICATE","duplicate class in one source unit"));
                }
                decls.push(decl);
                i=end;
                continue;
            }
        }
        if !entry && braces==0 {
            return Err(err(tokens[i].pos,"E_UNSUPPORTED",
                "dependency source units admit only class definitions and exports"));
        }
        if word(&tokens,i,"use") && braces==0 {
            return Err(err(tokens[i].pos,"E_UNSUPPORTED",
                "namespace syntax cannot override trusted build-owned unit identity"));
        }
        if tokens[i].tok==Tok::LBrace {braces+=1;}
        if tokens[i].tok==Tok::RBrace {braces=braces.saturating_sub(1);}
        i+=1;
    }
    for (name,vis) in &exports {
        let decl=decls.iter_mut().find(|d|&d.name==name).ok_or_else(||
            err(tokens.first().map_or(Pos {line:1,col:1},|t|t.pos),
                "E_UNBOUND","export must name a class declared in the same source unit"))?;
        decl.visibility=*vis;
    }
    Ok(Parsed {key,tokens:tokens[first_source..].to_vec(),imports,decls,
        emitted:Vec::new()})
}
fn identity(key: &UnitKey, name: &str) -> NominalId {
    NominalId {package:key.package.clone(),namespace:key.namespace.clone(),
        module:key.module.clone(),name:name.into()}
}
fn visible(from: &UnitKey, target: &LinkClass) -> bool {
    if target.id.package==from.package && target.id.namespace==from.namespace &&
       target.id.module==from.module {return true}
    match target.visibility {
        ClassVisibility::Public => true,
        ClassVisibility::Package => target.id.package==from.package,
        ClassVisibility::Private => false,
    }
}

/// Link validated source-units into one executable guest. The caller owns the
/// source catalogue and must authenticate those bytes and package identities
/// as part of its build; this API never accepts precompiled/forged metadata.
///
/// Entry unit contains the only top-level functions; dependencies may declare
/// classes and their methods. Each unit uses import class Remote [as Alias]
/// from "package/namespace/module"; before any declarations. Cyclic TYPE
/// imports are allowed, while unsupported runtime module imports, inherited
/// fields/constructors and unverified object dispatch fail closed.
pub fn compile_linked(units: &[SourceUnit<'_>], entry: usize) -> R<Vec<u8>> {
    if units.is_empty() || units.len()>MAX_UNITS || entry>=units.len() {
        return Err(err(Pos {line:1,col:1},"E_SOURCE_LIMIT","invalid linked source catalogue"));
    }
    let total=units.iter().map(|u|u.source.len()).sum::<usize>();
    if total>crate::SOURCE_CAP {
        return Err(err(Pos {line:1,col:1},"E_SOURCE_LIMIT",
            "linked source catalogue exceeds single executable budget"));
    }
    let mut by_key=BTreeMap::<UnitKey,Parsed>::new();
    for (i,unit) in units.iter().enumerate() {
        let parsed=parse(unit,i==entry)?;
        if by_key.insert(parsed.key.clone(),parsed).is_some() {
            return Err(err(Pos {line:1,col:1},"E_DUPLICATE",
                "duplicate build-owned package/namespace/module"));
        }
    }
    let entry_key=UnitKey::of(&units[entry]);
    let mut id_to_class=BTreeMap::<NominalId, LinkClass>::new();
    let mut ordinal=0usize;
    let mut mangled=BTreeMap::<NominalId,String>::new();
    for (key,parsed) in &by_key {
        for class in &parsed.decls {
            let id=identity(key,&class.name);
            mangled.insert(id.clone(),format!("{PREFIX}{ordinal}"));
            ordinal+=1;
            id_to_class.insert(id.clone(),LinkClass{
                id,pos:class.pos,visibility:class.visibility,hierarchy:class.kind,
                direct_parents:Vec::new(),permitted_children:Vec::new(),
            });
        }
    }
    if id_to_class.is_empty() {
        return Err(err(Pos {line:1,col:1},"E_UNSUPPORTED","linked profile requires at least one class"));
    }
    let mut symbol_tables=BTreeMap::<UnitKey,BTreeMap<String,NominalId>>::new();
    for (key,parsed) in &by_key {
        let mut symbols=BTreeMap::new();
        for class in &parsed.decls {
            symbols.insert(class.name.clone(),identity(key,&class.name));
        }
        for import in &parsed.imports {
            let remote=identity(&import.unit,&import.remote);
            let target=id_to_class.get(&remote).ok_or_else(||
                err(import.pos,"E_UNBOUND","import does not resolve to a class in the trusted source catalogue"))?;
            if !visible(key,target) {
                return Err(err(import.pos,"E_VISIBILITY",
                    "class import is inaccessible across the package/module boundary"));
            }
            if symbols.insert(import.local.clone(),remote).is_some() {
                return Err(err(import.pos,"E_DUPLICATE","class import alias shadows another class or import"));
            }
        }
        symbol_tables.insert(key.clone(),symbols);
    }
    for (key,parsed) in &by_key {
        let symbols=&symbol_tables[key];
        for class in &parsed.decls {
            let owner=identity(key,&class.name);
            let link=id_to_class.get_mut(&owner).expect("known own declaration");
            for parent in &class.parents {
                let id=symbols.get(parent).ok_or_else(||
                    err(class.pos,"E_UNBOUND","parent requires an exact local class or imported class binding"))?;
                link.direct_parents.push(id.clone());
            }
            for child in &class.permits {
                let id=symbols.get(child).ok_or_else(||
                    err(class.pos,"E_UNBOUND","permits requires an exact local class or imported class binding"))?;
                link.permitted_children.push(id.clone());
            }
        }
    }
    let graph:Vec<LinkClass>=id_to_class.values().cloned().collect();
    nominal_link::validate_prelink(&graph)?;

    for (key,parsed) in &mut by_key {
        let symbols=&symbol_tables[key];
        for (at,token) in parsed.tokens.iter().enumerate() {
            let mut adjusted=token.clone();
            if let Tok::Ident(name)=&token.tok {
                if let Some(id)=symbols.get(name) {
                    if at>0 && matches!(&parsed.tokens[at-1].tok, Tok::Dot) {
                        // Class spelling in a field/method position is not a type use.
                    } else {
                        if at>0 && (word(&parsed.tokens,at-1,"val") ||
                            word(&parsed.tokens,at-1,"let") ||
                            word(&parsed.tokens,at-1,"fnc") ||
                            word(&parsed.tokens,at-1,"method") ||
                            matches!(&parsed.tokens[at-1].tok,
                                Tok::Ident(t) if matches!(t.as_str(),
                                    "int" | "bool" | "float" | "string" | "void") ||
                                    symbols.contains_key(t))) {
                            return Err(err(token.pos,"E_NAMESPACE",
                                "class/import alias cannot be shadowed by a value or callable binder"));
                        }
                        adjusted.tok=Tok::Ident(mangled[id].clone());
                    }
                }
            }
            parsed.emitted.push(adjusted);
        }
    }
    let mut combined=Vec::new();
    for (key,parsed) in by_key {
        // A catalog entry not named as the root must not supply a second main.
        if key!=entry_key && parsed.emitted.iter().any(|t|word(std::slice::from_ref(t),0,"main")) {
            return Err(err(Pos {line:1,col:1},"E_UNSUPPORTED",
                "dependency class unit must not declare a root entry point"));
        }
        combined.extend(parsed.emitted);
    }
    if combined.len()+1>lexer::MAX_TOKENS {
        return Err(err(Pos {line:1,col:1},"E_SOURCE_LIMIT",
            "relocated compilation exceeds lexer token budget"));
    }
    combined.push(Token {tok:Tok::Eof,pos:Pos {line:1,col:1}});
    // The same full parser, nominal method-copy ABI checks, type/ownership
    // checker and guest emitter are now run on the relocated global class graph.
    crate::compile_token_stream(combined)
}

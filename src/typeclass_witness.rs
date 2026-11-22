//! Closed-world source-local scalar typeclasses for the browser compiler.
//! The compiler erases declarations and replaces explicit Class::member<T>
//! with a concrete, source-defined function or scalar type BEFORE parsing.
//! No runtime witnesses, vtables, reflection, operator overloads or imports.

use crate::diag::{Diagnostic, Pos};
use crate::lexer::{Tok, Token, MAX_TOKENS};
use std::collections::{BTreeMap, BTreeSet};

const MAX_CLASSES: usize = 16;
const MAX_IMPLS: usize = 64;
const MAX_MEMBERS: usize = 32;

#[derive(Clone)]
struct Method { name: String, params: Vec<String>, ret: String }
#[derive(Clone)]
struct Class { name: String, associated: Vec<String>, methods: Vec<Method> }
#[derive(Clone)]
struct Impl { class: String, ty: String, associated: BTreeMap<String, String>, methods: BTreeMap<String, String> }

fn error(pos: Pos, code: &'static str, message: &str) -> Diagnostic {
    Diagnostic::new(pos, code, message)
}
fn word(tokens: &[Token], at: usize) -> Option<&str> {
    match &tokens.get(at)?.tok { Tok::Ident(s) => Some(s), _ => None }
}
fn tok(tokens: &[Token], at: usize, expected: Tok) -> bool {
    tokens.get(at).is_some_and(|t| t.tok == expected)
}
fn ident(tokens: &[Token], at: &mut usize, label: &str) -> Result<String, Diagnostic> {
    let s = word(tokens, *at).ok_or_else(|| error(tokens.get(*at).map_or(Pos::default(), |t| t.pos),
        "E_WITNESS_SYNTAX", label))?.to_string();
    *at += 1;
    Ok(s)
}
fn expect(tokens: &[Token], at: &mut usize, wanted: Tok) -> Result<(), Diagnostic> {
    if !tok(tokens, *at, wanted) {
        return Err(error(tokens.get(*at).map_or(Pos::default(), |t| t.pos),
            "E_WITNESS_SYNTAX", "unexpected witness declaration token"));
    }
    *at += 1;
    Ok(())
}
fn keyword(tokens: &[Token], at: &mut usize, wanted: &str) -> Result<(), Diagnostic> {
    if word(tokens, *at) != Some(wanted) {
        return Err(error(tokens.get(*at).map_or(Pos::default(), |t| t.pos),
            "E_WITNESS_SYNTAX", "unexpected witness declaration keyword"));
    }
    *at += 1;
    Ok(())
}
fn scalar(s: &str) -> bool { matches!(s, "int" | "bool" | "float" | "string") }
fn identifier(s: &str) -> bool {
    !s.is_empty() && s.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_')
        && (s.as_bytes()[0].is_ascii_alphabetic() || s.as_bytes()[0] == b'_')
}
fn type_name(tokens: &[Token], at: &mut usize) -> Result<String, Diagnostic> {
    let s = ident(tokens, at, "expected scalar, Self or associated type")?;
    if !identifier(&s) { return Err(error(tokens[*at - 1].pos, "E_WITNESS_TYPE", "invalid type name")); }
    Ok(s)
}
fn parse_class(tokens: &[Token], at: &mut usize) -> Result<Class, Diagnostic> {
    keyword(tokens, at, "define")?;
    keyword(tokens, at, "typeclass")?;
    let name = ident(tokens, at, "typeclass name required")?;
    if !identifier(&name) || scalar(&name) || crate::parser::is_reserved(&name) {
        return Err(error(tokens[*at - 1].pos, "E_WITNESS_TYPE", "invalid typeclass name"));
    }
    let mut associated = Vec::new();
    let mut methods = Vec::new();
    loop {
        if *at >= tokens.len() || tok(tokens, *at, Tok::Eof) {
            return Err(error(tokens[(*at).min(tokens.len() - 1)].pos, "E_WITNESS_SYNTAX", "unclosed typeclass"));
        }
        if word(tokens, *at) == Some("end") { *at += 1; break; }
        match word(tokens, *at) {
            Some("type") => {
                *at += 1;
                let key = ident(tokens, at, "associated type name required")?;
                expect(tokens, at, Tok::Semi)?;
                if !identifier(&key) || scalar(&key) || key == "Self" || associated.contains(&key)
                    || associated.len() == MAX_MEMBERS {
                    return Err(error(tokens[*at - 1].pos, "E_WITNESS_ASSOC", "duplicate or invalid associated type"));
                }
                associated.push(key);
            }
            Some("fnc") => {
                *at += 1;
                let method = ident(tokens, at, "typeclass method name required")?;
                expect(tokens, at, Tok::LParen)?;
                let mut params = Vec::new();
                if !tok(tokens, *at, Tok::RParen) {
                    loop {
                        params.push(type_name(tokens, at)?);
                        if params.len() > 32 { return Err(error(tokens[*at-1].pos, "E_WITNESS_LIMIT", "too many parameters")); }
                        if !tok(tokens, *at, Tok::Comma) { break; }
                        *at += 1;
                    }
                }
                expect(tokens, at, Tok::RParen)?;
                expect(tokens, at, Tok::Colon)?;
                let ret = type_name(tokens, at)?;
                expect(tokens, at, Tok::Semi)?;
                if !identifier(&method) || methods.iter().any(|m: &Method| m.name == method)
                    || methods.len() == MAX_MEMBERS {
                    return Err(error(tokens[*at-1].pos, "E_WITNESS_METHOD", "duplicate or invalid typeclass method"));
                }
                methods.push(Method { name: method, params, ret });
            }
            _ => return Err(error(tokens[*at].pos, "E_WITNESS_SYNTAX", "typeclass allows only type and fnc entries")),
        }
    }
    if methods.is_empty() { return Err(error(tokens[*at-1].pos, "E_WITNESS_METHOD", "typeclass must declare a method")); }
    for m in &methods {
        for s in m.params.iter().chain(std::iter::once(&m.ret)) {
            if !scalar(s) && s != "Self" && s != "void" && !associated.contains(s) {
                return Err(error(tokens[*at-1].pos, "E_WITNESS_TYPE", "undeclared signature type"));
            }
        }
    }
    Ok(Class { name, associated, methods })
}
fn parse_impl(tokens: &[Token], at: &mut usize) -> Result<Impl, Diagnostic> {
    keyword(tokens, at, "define")?;
    keyword(tokens, at, "impl")?;
    let class = ident(tokens, at, "typeclass expected")?;
    keyword(tokens, at, "for")?;
    let ty = type_name(tokens, at)?;
    if !scalar(&ty) { return Err(error(tokens[*at-1].pos, "E_WITNESS_TYPE", "only scalar implementations are executable")); }
    let mut associated = BTreeMap::new();
    let mut methods = BTreeMap::new();
    loop {
        if *at >= tokens.len() || tok(tokens, *at, Tok::Eof) {
            return Err(error(tokens[(*at).min(tokens.len()-1)].pos, "E_WITNESS_SYNTAX", "unclosed implementation"));
        }
        if word(tokens, *at) == Some("end") { *at += 1; break; }
        match word(tokens, *at) {
            Some("type") => {
                *at += 1;
                let key = ident(tokens, at, "associated type required")?;
                expect(tokens, at, Tok::Assign)?;
                let value = type_name(tokens, at)?;
                expect(tokens, at, Tok::Semi)?;
                if !(scalar(&value) || value == "Self") || associated.insert(key, value).is_some() {
                    return Err(error(tokens[*at-1].pos, "E_WITNESS_ASSOC", "invalid or duplicate associated binding"));
                }
            }
            Some("bind") => {
                *at += 1;
                let method = ident(tokens, at, "method required")?;
                expect(tokens, at, Tok::Assign)?;
                let target = ident(tokens, at, "function target required")?;
                expect(tokens, at, Tok::Semi)?;
                if !identifier(&target) || methods.insert(method, target).is_some() {
                    return Err(error(tokens[*at-1].pos, "E_WITNESS_BINDING", "invalid or duplicate binding"));
                }
            }
            _ => return Err(error(tokens[*at].pos, "E_WITNESS_SYNTAX", "implementation allows only type and bind entries")),
        }
        if associated.len() > MAX_MEMBERS || methods.len() > MAX_MEMBERS {
            return Err(error(tokens[*at-1].pos, "E_WITNESS_LIMIT", "too many implementation entries"));
        }
    }
    Ok(Impl { class, ty, associated, methods })
}
fn substituted(name: &str, imp: &Impl) -> Option<String> {
    if name == "Self" { return Some(imp.ty.clone()); }
    if name == "void" || scalar(name) { return Some(name.to_string()); }
    imp.associated.get(name).map(|s| if s == "Self" { imp.ty.clone() } else { s.clone() })
}
fn concrete_signatures(tokens: &[Token]) -> Result<BTreeMap<String, (Vec<String>, String)>, Diagnostic> {
    let mut defs = BTreeMap::new();
    let mut depth = 0usize;
    let mut i = 0;
    while i < tokens.len() {
        if tok(tokens, i, Tok::LBrace) { depth += 1; }
        if tok(tokens, i, Tok::RBrace) { depth = depth.saturating_sub(1); }
        if depth == 0 && word(tokens, i) == Some("fnc") {
            if let Some(name) = word(tokens, i+1) {
                let name = name.to_string();
                if tok(tokens, i+2, Tok::LParen) {
                    let mut j = i+3;
                    let mut params = Vec::new();
                    while !tok(tokens, j, Tok::RParen) {
                        if j >= tokens.len() || tok(tokens, j, Tok::Eof) {
                            return Err(error(tokens[i].pos, "E_WITNESS_SIGNATURE", "unclosed target parameter list"));
                        }
                        let first = word(tokens, j).unwrap_or("");
                        let second = word(tokens, j+1).unwrap_or("");
                        let ty = if scalar(first) && !second.is_empty() {
                            j += 2; first.to_string()
                        } else if !first.is_empty() && tok(tokens, j+1, Tok::Colon)
                            && scalar(word(tokens, j+2).unwrap_or("")) {
                            j += 3; word(tokens, j-1).unwrap().to_string()
                        } else {
                            params.clear(); break;
                        };
                        params.push(ty);
                        if tok(tokens, j, Tok::Comma) { j += 1; }
                        else if !tok(tokens, j, Tok::RParen) { params.clear(); break; }
                    }
                    if tok(tokens, j, Tok::RParen) && tok(tokens, j+1, Tok::Colon) {
                        if let Some(ret) = word(tokens, j+2) {
                            if scalar(ret) || ret == "void" {
                                if defs.insert(name, (params, ret.to_string())).is_some() {
                                    return Err(error(tokens[i].pos, "E_WITNESS_AMBIGUOUS", "target overloading is not allowed"));
                                }
                            }
                        }
                    }
                }
            }
        }
        i += 1;
    }
    Ok(defs)
}

/// Lower an entire already-lexed source file without allocating a runtime
/// witness. The ordinary parser and typed checker still own guest validation.
pub fn lower(tokens: Vec<Token>) -> Result<Vec<Token>, Diagnostic> {
    let mut classes = BTreeMap::<String, Class>::new();
    let mut impls = BTreeMap::<(String, String), Impl>::new();
    let mut output = Vec::new();
    let mut at = 0;
    let mut braces = 0usize;
    while at < tokens.len() {
        if word(&tokens, at) == Some("define") && matches!(word(&tokens, at+1), Some("typeclass" | "impl")) {
            if braces != 0 { return Err(error(tokens[at].pos, "E_WITNESS_SCOPE", "declaration must be file-scoped")); }
            let pos = tokens[at].pos;
            if word(&tokens, at+1) == Some("typeclass") {
                let c = parse_class(&tokens, &mut at)?;
                if classes.len() >= MAX_CLASSES || classes.insert(c.name.clone(), c).is_some() {
                    return Err(error(pos, "E_WITNESS_AMBIGUOUS", "duplicate or excessive typeclasses"));
                }
            } else {
                let imp = parse_impl(&tokens, &mut at)?;
                if impls.len() >= MAX_IMPLS || impls.insert((imp.class.clone(), imp.ty.clone()), imp).is_some() {
                    return Err(error(pos, "E_WITNESS_AMBIGUOUS", "duplicate or excessive implementations"));
                }
            }
            continue;
        }
        if tok(&tokens, at, Tok::LBrace) { braces += 1; }
        if tok(&tokens, at, Tok::RBrace) { braces = braces.saturating_sub(1); }
        output.push(tokens[at].clone());
        at += 1;
    }
    if classes.is_empty() && impls.is_empty() { return Ok(tokens); }
    if classes.is_empty() { return Err(error(tokens[0].pos, "E_WITNESS_MISSING", "implementation without a local typeclass")); }
    let sigs = concrete_signatures(&output)?;
    let mut reserved = BTreeSet::new();
    for (name, class) in &classes {
        reserved.insert(name.clone());
        for ((owner, _), imp) in impls.iter().filter(|((owner, _), _)| owner == name) {
            if owner != name { unreachable!(); }
            if imp.associated.len() != class.associated.len()
                || class.associated.iter().any(|k| !imp.associated.contains_key(k))
                || imp.methods.len() != class.methods.len()
                || class.methods.iter().any(|m| !imp.methods.contains_key(&m.name)) {
                return Err(error(tokens[0].pos, "E_WITNESS_INCOMPLETE", "all associated types and methods must be bound exactly"));
            }
            for method in &class.methods {
                let symbol = &imp.methods[&method.name];
                if reserved.contains(symbol) { return Err(error(tokens[0].pos, "E_WITNESS_BINDING", "target collides with typeclass")); }
                let Some((params, ret)) = sigs.get(symbol) else {
                    return Err(error(tokens[0].pos, "E_WITNESS_BINDING", "witness target must be a local scalar fnc"));
                };
                let expected: Option<Vec<_>> = method.params.iter().map(|p| substituted(p, imp)).collect();
                if expected.as_ref() != Some(params) || substituted(&method.ret, imp).as_ref() != Some(ret) {
                    return Err(error(tokens[0].pos, "E_WITNESS_SIGNATURE", "target signature differs from requirement"));
                }
            }
        }
    }
    for ((name, _), _) in &impls {
        if !classes.contains_key(name) {
            return Err(error(tokens[0].pos, "E_WITNESS_ORPHAN", "implementation needs a source-owned typeclass"));
        }
    }
    let mut lowered = Vec::new();
    let mut i = 0;
    while i < output.len() {
        let Some(owner) = word(&output, i) else { lowered.push(output[i].clone()); i += 1; continue; };
        let Some(class) = classes.get(owner) else { lowered.push(output[i].clone()); i += 1; continue; };
        if !tok(&output, i+1, Tok::ColonColon) {
            return Err(error(output[i].pos, "E_WITNESS_VALUE", "typeclasses have no runtime values"));
        }
        let member = word(&output, i+2).ok_or_else(|| error(output[i].pos, "E_WITNESS_SYNTAX", "member required"))?;
        if !tok(&output, i+3, Tok::Lt) || !tok(&output, i+5, Tok::Gt) {
            return Err(error(output[i].pos, "E_WITNESS_INFER", "explicit one-scalar specialization required"));
        }
        let ty = word(&output, i+4).ok_or_else(|| error(output[i].pos, "E_WITNESS_TYPE", "scalar type required"))?;
        if !scalar(ty) { return Err(error(output[i].pos, "E_WITNESS_TYPE", "only scalar implementations supported")); }
        let imp = impls.get(&(owner.to_string(), ty.to_string())).ok_or_else(||
            error(output[i].pos, "E_WITNESS_MISSING", "no coherent implementation for concrete type"))?;
        let concrete = if class.associated.iter().any(|a| a == member) {
            substituted(member, imp).ok_or_else(|| error(output[i].pos, "E_WITNESS_ASSOC", "unbound associated type"))?
        } else if class.methods.iter().any(|m| m.name == member) {
            if !tok(&output, i+6, Tok::LParen) {
                return Err(error(output[i].pos, "E_WITNESS_VALUE", "method witness cannot escape as a value"));
            }
            imp.methods.get(member).ok_or_else(|| error(output[i].pos, "E_WITNESS_BINDING", "unbound method"))?.clone()
        } else {
            return Err(error(output[i].pos, "E_WITNESS_MEMBER", "unknown member"));
        };
        lowered.push(Token { tok: Tok::Ident(concrete), pos: output[i].pos });
        i += 6;
    }
    if lowered.len() > MAX_TOKENS {
        return Err(error(tokens[0].pos, "E_WITNESS_LIMIT", "rewritten source exceeds token budget"));
    }
    Ok(lowered)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lexer::lex;

    const DECL: &str = r#"
        define typeclass Additive
            type Output;
            fnc add(Self, Self): Output;
        end
        define impl Additive for int
            type Output = int;
            bind add = add_int;
        end
        fnc add_int(int a, int b): int { return a + b; }
    "#;
    #[test]
    fn direct_symbol_and_associated_type_are_erased_before_codegen() {
        let src = format!("{DECL} pub fnc main(): int {{ val x: Additive::Output<int> = Additive::add<int>(20,22); return x; }}");
        let toks = lower(lex(&src).unwrap()).unwrap();
        let words: Vec<_> = toks.iter().filter_map(|t| match &t.tok { Tok::Ident(s)=>Some(s.as_str()),_=>None }).collect();
        assert!(words.contains(&"add_int"));
        assert!(!words.contains(&"Additive"));
        assert!(!words.contains(&"bind"));
        assert_eq!(toks.last().map(|t| &t.tok), Some(&Tok::Eof));
    }
    #[test]
    fn incomplete_duplicate_mismatch_missing_and_escape_fail_closed() {
        for (src, expected) in [
            (format!("{DECL} define impl Additive for int type Output=int; bind add=add_int; end"), "E_WITNESS_AMBIGUOUS"),
            (DECL.replace("bind add = add_int;", ""), "E_WITNESS_INCOMPLETE"),
            (DECL.replace("fnc add_int(int a, int b): int", "fnc add_int(bool a, int b): int"), "E_WITNESS_SIGNATURE"),
            (format!("{DECL} pub fnc main(): int {{ return Additive::add<bool>(1,2); }}"), "E_WITNESS_MISSING"),
            (format!("{DECL} pub fnc main(): int {{ val x = Additive; return 0; }}"), "E_WITNESS_VALUE"),
            (DECL.replace("bind add = add_int;", "bind add = unknown;"), "E_WITNESS_BINDING"),
        ] {
            let err = lower(lex(&src).unwrap()).err().expect(&src);
            assert_eq!(err.code, expected, "{src}");
        }
    }
    #[test]
    fn guest_wasm_is_produced_from_a_static_direct_call() {
        let src = format!("{DECL} pub fnc main(): int {{ val x: Additive::Output<int> = Additive::add<int>(20,22); return x; }}");
        let bytes = crate::compile(&src).unwrap();
        assert!(bytes.starts_with(b"\0asm"));
    }
}

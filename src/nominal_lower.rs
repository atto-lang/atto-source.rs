//! Conservative nominal composition for source-local interface/trait/class declarations.
//!
//! Interface obligations and concrete trait/class methods are resolved before
//! the production parser, checker and Wasm emitter. This profile has NO runtime
//! base-class pointer, implicit dynamic dispatch, inherited fields, or unsafe
//! actor state. All collisions, cycles and missing methods fail closed.
use std::collections::{BTreeMap, BTreeSet};

use crate::diag::{Diagnostic, Pos};
use crate::lexer::{Tok, Token};

type R<T> = Result<T, Diagnostic>;
const MAX_DECLS: usize = 96;
const MAX_METHODS: usize = 256;
const MAX_BASES: usize = 8;

#[derive(Clone, Copy, Eq, PartialEq)]
enum Kind { Class, Trait, Interface }
#[derive(Clone)]
struct Method {
    name: String,
    sig: Vec<Tok>,
    tokens: Vec<Token>,
    origin: String,
    public: bool,
    overriding: bool,
    /// Exact source position of the override keyword; body identifiers
    /// named 'override' must never be rewritten or dropped.
    override_pos: Option<Pos>,
    pos: Pos,
}
#[derive(Clone)]
struct Decl {
    name: String,
    pos: Pos,
    kind: Kind,
    start: usize,
    end: usize,
    body: Vec<Token>,
    parents: Vec<String>,
    contracts: Vec<String>,
    methods: BTreeMap<String, Method>,
    has_fields_or_ctor: bool,
    export: bool,
    requires_lowering: bool,
}
fn is(t: &[Token], i: usize, value: &str) -> bool {
    matches!(t.get(i).map(|t| &t.tok), Some(Tok::Ident(s)) if s == value)
}
fn name(t: &[Token], i: usize) -> R<String> {
    match t.get(i) {
        Some(Token { tok: Tok::Ident(s), pos }) if s.len() <= 96 &&
            !matches!(s.as_str(), "as" | "end" | "define" | "pub" | "class" | "trait" | "interface" | "extends" | "implements") =>
            Ok(s.clone()),
        Some(t) => Err(Diagnostic::new(t.pos, "E_SYNTAX", "expected a bounded nominal identifier")),
        None => Err(Diagnostic::new(Pos { line: 1, col: 1 }, "E_EOF", "missing nominal identifier")),
    }
}
fn diag(pos: Pos, code: &'static str, what: impl Into<String>) -> Diagnostic {
    Diagnostic::new(pos, code, what)
}
fn closing(t: &[Token], open: usize, left: Tok, right: Tok) -> R<usize> {
    let mut depth = 0usize;
    for i in open..t.len() {
        if t[i].tok == left { depth += 1; }
        if t[i].tok == right {
            if depth == 0 { return Err(diag(t[i].pos, "E_SYNTAX", "unbalanced delimiter")); }
            depth -= 1;
            if depth == 0 { return Ok(i); }
        }
        if depth > 128 { return Err(diag(t[i].pos, "E_NESTING", "nominal member nesting limit")); }
    }
    Err(diag(t[open].pos, "E_EOF", "unclosed nominal method body"))
}

/// Parse only brace-bodied methods and interface signatures. Refuse exotic
/// member syntax rather than accidentally treating a declaration as a field.
fn members(t: &[Token], kind: Kind, owner: &str, has_parent: bool) -> R<(BTreeMap<String, Method>, bool)> {
    let (mut i, mut out, mut fields) = (0usize, BTreeMap::new(), false);
    while i < t.len() {
        if out.len() >= MAX_METHODS { return Err(diag(t[i].pos, "E_SOURCE_LIMIT", "too many nominal members")); }
        let start = i;
        let public = if is(t, i, "pub") { i += 1; true } else {
            if is(t, i, "private") { i += 1; }
            kind != Kind::Class
        };
        let override_at = if is(t, i, "override") { let v = i; i += 1; Some(v) } else { None };
        if override_at.is_some() && kind != Kind::Class {
            return Err(diag(t[start].pos, "E_OVERRIDE",
                "trait/interface methods cannot declare a class override"));
        }
        // Parsed only in a class member declaration. Until inherited physical
        // slots have a verified layout ABI, recognize the exact target spelling
        // and fail closed rather than treating `use` as a field declaration.
        // A normal method named `use(...)` is NOT a field-alias declaration.
        if is(t, i, "use") && t.get(i + 1).map(|x| &x.tok) != Some(&Tok::LParen) {
            let at = t[i].pos;
            if kind != Kind::Class || override_at.is_some() || public {
                return Err(diag(at, "E_SYNTAX", "inherited field aliases require a plain class member 'use' declaration"));
            }
            let ancestor = name(t, i + 1)?;
            if t.get(i + 2).map(|x| &x.tok) != Some(&Tok::Dot) {
                return Err(diag(t.get(i + 2).map_or(at, |x| x.pos), "E_SYNTAX",
                    "inherited member qualification uses '.' (never '::')"));
            }
            let field = name(t, i + 3)?;
            if !is(t, i + 4, "as") {
                return Err(diag(t.get(i + 4).map_or(at, |x| x.pos), "E_SYNTAX",
                    "inherited field alias requires 'as alias_name'"));
            }
            let alias = name(t, i + 5)?;
            if t.get(i + 6).map(|x| &x.tok) != Some(&Tok::Semi) {
                return Err(diag(t.get(i + 6).map_or(at, |x| x.pos), "E_SYNTAX",
                    "inherited field alias must end with ';'"));
            }
            if !has_parent {
                return Err(diag(at, "E_ALIAS_TARGET", format!(
                    "cannot alias {ancestor}.{field} as {alias}: class has no ancestors")));
            }
            // Syntax is valid. Semantic alias admission and physical-slot
            // generation run after the complete nominal ancestry is known.
            i += 7;
            continue;
        }
        if is(t, i, "fnc") || is(t, i, "method") { i += 1; }
        let is_ctor = is(t, i, "constructor");
        let method = matches!(t.get(i + 1).map(|x| &x.tok), Some(Tok::LParen));
        if !method && !is_ctor {
            if kind != Kind::Class { return Err(diag(t[start].pos, "E_UNSUPPORTED", "interfaces/traits only admit callable members")); }
            if override_at.is_some() {
                return Err(diag(t[start].pos, "E_OVERRIDE", "override requires a callable method"));
            }
            fields = true;
            while i < t.len() && t[i].tok != Tok::Semi {
                if t[i].tok == Tok::LBrace || t[i].tok == Tok::Eof {
                    return Err(diag(t[i].pos, "E_UNSUPPORTED", "non-scalar inherited fields are unsupported"));
                }
                i += 1;
            }
            if i == t.len() { return Err(diag(t[start].pos, "E_SYNTAX", "unterminated class field")); }
            i += 1;
            continue;
        }
        let member_name = name(t, i)?;
        if is_ctor && kind != Kind::Class {
            return Err(diag(t[i].pos, "E_UNSUPPORTED", "trait/interface constructors are unsupported"));
        }
        let args_end = closing(t, i + 1, Tok::LParen, Tok::RParen)?;
        let mut end = args_end + 1;
        while end < t.len() && t[end].tok != Tok::LBrace && t[end].tok != Tok::Semi {
            if is(t, end, "end") { return Err(diag(t[end].pos, "E_UNSUPPORTED", "end-bodied inherited members require separate lowering")); }
            end += 1;
        }
        if end == t.len() { return Err(diag(t[start].pos, "E_EOF", "unterminated member signature")); }
        let signature = t[i + 1..end].iter().map(|t| t.tok.clone()).collect();
        let header_end = end;
        let has_body = t[end].tok == Tok::LBrace;
        if kind == Kind::Interface && has_body {
            return Err(diag(t[end].pos, "E_UNSUPPORTED", "interface members require signatures, not trait defaults"));
        }
        if kind != Kind::Interface && !has_body {
            return Err(diag(t[end].pos, "E_UNSUPPORTED", "concrete nominal members require braced implementation"));
        }
        if has_body { end = closing(t, end, Tok::LBrace, Tok::RBrace)?; }
        if is_ctor {
            fields = true;
            i = end + 1;
            continue;
        }
        let mut copied = t[start..=end].to_vec();
        if let Some(at) = override_at { copied.remove(at - start); }
        if kind == Kind::Trait && !is(t, start, "pub") {
            copied.insert(0, Token { pos: t[start].pos, tok: Tok::Ident("pub".into()) });
        }
        let m = Method {
            name: member_name.clone(), sig: signature,
            tokens: copied, origin: format!("{owner}::{member_name}"),
            public, overriding: override_at.is_some(),
            override_pos: override_at.map(|i| t[i].pos), pos: t[start].pos,
        };
        if out.insert(member_name.clone(), m).is_some() {
            return Err(diag(t[start].pos, "E_DUPLICATE", format!("duplicate method '{member_name}'")));
        }
        i = header_end;
        if has_body { i = end + 1; } else { i += 1; }
    }
    Ok((out, fields))
}
fn decls(t: &[Token]) -> R<BTreeMap<String, Decl>> {
    let (mut i, mut out, mut braces) = (0usize, BTreeMap::new(), 0usize);
    while i < t.len() {
        if t[i].tok == Tok::LBrace { braces += 1; }
        if t[i].tok == Tok::RBrace { braces = braces.saturating_sub(1); }
        if braces > 0 { i += 1; continue; }
        let start = i;
        let mut j = i;
        let mut export = false;
        if is(t, j, "pub") && is(t, j + 1, "define") { export = true; j += 1; }
        if !is(t, j, "define") { i += 1; continue; }
        j += 1;
        if is(t, j, "pub") { export = true; j += 1; }
        let kind = if is(t, j, "class") { Kind::Class }
            else if is(t, j, "trait") { Kind::Trait }
            else if is(t, j, "interface") { Kind::Interface }
            else { i += 1; continue; };
        j += 1;
        if out.len() >= MAX_DECLS { return Err(diag(t[start].pos, "E_SOURCE_LIMIT", "too many nominal definitions")); }
        let owner = name(t, j)?;
        j += 1;
        let mut parents = Vec::new();
        let mut contracts = Vec::new();
        if is(t, j, "extends") {
            j += 1;
            loop {
                if parents.len() >= MAX_BASES { return Err(diag(t[start].pos, "E_SOURCE_LIMIT", "too many class bases")); }
                let candidate = name(t, j)?;
                if parents.contains(&candidate) {
                    return Err(diag(t[j].pos, "E_DUPLICATE",
                        format!("duplicate class base '{candidate}'")));
                }
                parents.push(candidate); j += 1;
                if t.get(j).map(|x| &x.tok) != Some(&Tok::Comma) { break; }
                j += 1;
            }
        }
        if is(t, j, "implements") {
            j += 1;
            loop {
                if contracts.len() >= MAX_BASES { return Err(diag(t[start].pos, "E_SOURCE_LIMIT", "too many contracts")); }
                let candidate = name(t, j)?;
                if contracts.contains(&candidate) {
                    return Err(diag(t[j].pos, "E_DUPLICATE",
                        format!("duplicate implemented contract '{candidate}'")));
                }
                contracts.push(candidate); j += 1;
                if t.get(j).map(|x| &x.tok) != Some(&Tok::Comma) { break; }
                j += 1;
            }
        }
        if !is(t, j, "as") { return Err(diag(t[j.min(t.len()-1)].pos, "E_SYNTAX", "expected 'as' after nominal header")); }
        j += 1;
        let body_start = j;
        let mut depth = 0usize;
        while j < t.len() {
            if t[j].tok == Tok::LBrace { depth += 1; }
            if t[j].tok == Tok::RBrace {
                if depth == 0 { return Err(diag(t[j].pos, "E_SYNTAX", "orphan class member brace")); }
                depth -= 1;
            }
            if depth == 0 && is(t, j, "end") { break; }
            if t[j].tok == Tok::Eof { break; }
            j += 1;
        }
        if j == t.len() || t[j].tok == Tok::Eof {
            return Err(diag(t[start].pos, "E_EOF", "unterminated nominal declaration"));
        }
        let body = t[body_start..j].to_vec();
        let (methods, has_fields_or_ctor) = members(&body, kind, &owner, !parents.is_empty())?;
        let d = Decl {
            name: owner.clone(), pos: t[start].pos, kind, start, end: j + 1,
            body, parents: parents.clone(), contracts: contracts.clone(),
            methods, has_fields_or_ctor, export,
            requires_lowering: kind != Kind::Class || !parents.is_empty() || !contracts.is_empty(),
        };
        if out.insert(owner.clone(), d).is_some() {
            return Err(diag(t[start].pos, "E_DUPLICATE", format!("duplicate nominal declaration '{owner}'")));
        }
        i = j + 1;
    }
    Ok(out)
}

#[derive(Clone)]
struct ScalarBaseField {
    name: String,
    tokens: Vec<Token>,
    name_index: usize,
    public: bool,
}
#[derive(Clone)]
struct InheritedAlias {
    parent: String,
    field: String,
    alias: String,
    start: usize,
    end: usize,
    pos: Pos,
}

/// Admit only initialized, directly owned int/bool storage. Any constructor,
/// method body, computed initializer, nested object or opaque source syntax
/// fails closed rather than silently dropping a parent's storage.
fn scalar_base_fields(base: &Decl) -> R<Vec<ScalarBaseField>> {
    if !base.parents.is_empty() || !base.contracts.is_empty() || !base.methods.is_empty() {
        return Err(diag(base.pos, "E_UNSUPPORTED",
            "state-bearing inheritance requires direct bases without methods or ancestors"));
    }
    let t = &base.body;
    let mut i = 0;
    let mut fields = Vec::new();
    let mut names = BTreeSet::new();
    while i < t.len() {
        if fields.len() >= 64 {
            return Err(diag(t[i].pos, "E_SOURCE_LIMIT", "too many inherited scalar fields"));
        }
        let begin = i;
        let public = is(t, i, "pub");
        if public || is(t, i, "private") { i += 1; }
        let kind = if is(t, i, "int") { "int" } else if is(t, i, "bool") { "bool" } else {
            return Err(diag(t[begin].pos, "E_UNSUPPORTED",
                "inherited state admits only initialized int/bool fields, without constructors"));
        };
        i += 1;
        let field_name = name(t, i)?;
        let at_name = i - begin;
        i += 1;
        if t.get(i).map(|v| &v.tok) != Some(&Tok::Colon) {
            return Err(diag(t[begin].pos, "E_UNSUPPORTED",
                "inherited scalar fields require a literal ':' initializer"));
        }
        i += 1;
        if kind == "int" && t.get(i).map(|v| &v.tok) == Some(&Tok::Minus) { i += 1; }
        let valid_literal = match t.get(i).map(|v| &v.tok) {
            Some(Tok::Int(_)) if kind == "int" => true,
            Some(Tok::Ident(v)) if kind == "bool" && (v == "true" || v == "false") => true,
            _ => false,
        };
        if !valid_literal {
            return Err(diag(t.get(i).map_or(t[begin].pos, |v| v.pos), "E_UNSUPPORTED",
                "inherited scalar initializers must be literal int/bool values"));
        }
        i += 1;
        if t.get(i).map(|v| &v.tok) != Some(&Tok::Semi) {
            return Err(diag(t.get(i).map_or(t[begin].pos, |v| v.pos), "E_UNSUPPORTED",
                "inherited scalar field must be terminated by ';'"));
        }
        i += 1;
        if !names.insert(field_name.clone()) {
            return Err(diag(t[begin].pos, "E_DUPLICATE", "duplicate inherited field name"));
        }
        fields.push(ScalarBaseField {
            name: field_name, tokens: t[begin..i].to_vec(),
            name_index: at_name, public,
        });
    }
    if fields.is_empty() {
        return Err(diag(base.pos, "E_UNSUPPORTED", "base constructor/state layout is not supported"));
    }
    Ok(fields)
}

/// Alias declarations have no storage identity. Each exact direct-base field
/// is represented by ONE physical child-class slot; the alias only chooses the
/// source-level name of that slot. No extra field is emitted per alias.
fn alias_declarations(d: &Decl) -> R<Vec<InheritedAlias>> {
    let t = &d.body;
    let mut aliases = Vec::new();
    let mut i = 0;
    while i < t.len() {
        if t[i].tok == Tok::LBrace {
            i = closing(t, i, Tok::LBrace, Tok::RBrace)? + 1;
            continue;
        }
        if is(t, i, "use") && t.get(i + 1).map(|v| &v.tok) != Some(&Tok::LParen) {
            // The structural spelling was checked by members(). Never
            // reinterpret an ordinary method named use() as an alias.
            if i + 6 >= t.len() || t[i + 2].tok != Tok::Dot
                || !is(t, i + 4, "as") || t[i + 6].tok != Tok::Semi {
                return Err(diag(t[i].pos, "E_SYNTAX", "malformed inherited field alias"));
            }
            aliases.push(InheritedAlias {
                parent: name(t, i + 1)?,
                field: name(t, i + 3)?,
                alias: name(t, i + 5)?,
                start: i, end: i + 7, pos: t[i].pos,
            });
            i += 7;
        } else { i += 1; }
    }
    Ok(aliases)
}

fn checked_scalar_aliases(
    d: &Decl, all: &BTreeMap<String, Decl>, aliases: &[InheritedAlias],
) -> R<Vec<Token>> {
    let mut parent_fields = BTreeMap::<(String, String), ScalarBaseField>::new();
    let mut physical_order = Vec::new();
    let mut exposed_names = BTreeSet::new();
    let mut any_state = false;
    for parent in &d.parents {
        let base = all.get(parent).ok_or_else(||
            diag(d.pos, "E_UNBOUND", format!("unknown parent class '{parent}'")))?;
        if base.kind != Kind::Class {
            return Err(diag(d.pos, "E_TYPE", "extends requires a concrete class"));
        }
        if base.has_fields_or_ctor {
            any_state = true;
            let fields = scalar_base_fields(base)?;
            for f in fields {
                exposed_names.insert(f.name.clone());
                let key = (parent.clone(), f.name.clone());
                physical_order.push(key.clone());
                parent_fields.insert(key, f);
            }
        }
        // Field-alias names may not shadow inherited method namespaces.
        for key in base.methods.keys() { exposed_names.insert(key.clone()); }
    }
    if !any_state && !aliases.is_empty() {
        return Err(diag(aliases[0].pos, "E_ALIAS_TARGET",
            "field alias target must be an inherited instance field"));
    }
    if !any_state { return Ok(Vec::new()); }
    if d.has_fields_or_ctor || !d.contracts.is_empty() {
        return Err(diag(d.pos, "E_UNSUPPORTED",
            "state-bearing inheritance with local fields/constructors or traits is not implemented"));
    }
    let mut used_origins = BTreeSet::new();
    let mut alias_names = BTreeSet::new();
    let mut aliases_by_origin = BTreeMap::new();
    for alias in aliases {
        if !d.parents.contains(&alias.parent) {
            return Err(diag(alias.pos, "E_ALIAS_TARGET",
                "field alias must select a direct ancestor in the bounded scalar layout"));
        }
        let key = (alias.parent.clone(), alias.field.clone());
        let original = parent_fields.get(&key).ok_or_else(||
            diag(alias.pos, "E_ALIAS_TARGET", format!(
                "no inherited instance field '{}.{}'", alias.parent, alias.field)))?;
        if !original.public {
            return Err(diag(alias.pos, "E_ALIAS_VISIBILITY",
                "private parent fields cannot be re-exported through use"));
        }
        if !used_origins.insert(key.clone()) || !alias_names.insert(alias.alias.clone())
            || exposed_names.contains(&alias.alias) || d.methods.contains_key(&alias.alias) {
            return Err(diag(alias.pos, "E_ALIAS_DUPLICATE",
                "field alias duplicates an origin or collides with an inherited/local member"));
        }
        aliases_by_origin.insert(key, alias);
    }
    if used_origins.len() != parent_fields.len() {
        return Err(diag(d.pos, "E_UNSUPPORTED",
            "every inherited physical scalar slot requires exactly one explicit use alias"));
    }
    // Parent declaration order and field declaration order, never alias
    // textual order, determine physical AOT slot layout.
    let mut output = Vec::new();
    for key in physical_order {
        let original = &parent_fields[&key];
        let alias = aliases_by_origin[&key];
        let mut field_tokens = original.tokens.clone();
        field_tokens[original.name_index] = Token {
            tok: Tok::Ident(alias.alias.clone()), pos: alias.pos,
        };
        output.extend(field_tokens);
    }
    Ok(output)
}
fn resolve(
    owner: &str, all: &BTreeMap<String, Decl>,
    cached: &mut BTreeMap<String, BTreeMap<String, Method>>,
    visiting: &mut BTreeSet<String>,
) -> R<BTreeMap<String, Method>> {
    if let Some(found) = cached.get(owner) { return Ok(found.clone()); }
    let d = all.get(owner).ok_or_else(|| diag(Pos { line: 1, col: 1 }, "E_UNBOUND", format!("unknown class '{owner}'")))?;
    if d.kind != Kind::Class { return Err(diag(d.pos, "E_TYPE", "extends requires a concrete class")); }
    if !visiting.insert(owner.to_owned()) { return Err(diag(d.pos, "E_CYCLE", "class inheritance cycle")); }
    let answer = (|| -> R<BTreeMap<String, Method>> {
        let mut inherited = BTreeMap::<String, Vec<Method>>::new();
        for parent in &d.parents {
            let base = all.get(parent).ok_or_else(|| diag(d.pos, "E_UNBOUND", format!("unknown class '{parent}'")))?;
            if base.kind != Kind::Class { return Err(diag(d.pos, "E_TYPE", "extends requires a class, not a trait/interface")); }
            // Inherited state is admitted separately by checked_scalar_aliases,
            // which rejects constructors, transitive state and unknown layouts.
            for (key, method) in resolve(parent, all, cached, visiting)? {
                if method.public {
                    let same = inherited.entry(key).or_default();
                    if !same.iter().any(|m| m.origin == method.origin) { same.push(method); }
                }
            }
        }
        let mut requirements = BTreeMap::<String, Vec<Tok>>::new();
        for contract in &d.contracts {
            let trait_or_if = all.get(contract).ok_or_else(|| diag(d.pos, "E_UNBOUND", format!("unknown implemented contract '{contract}'")))?;
            match trait_or_if.kind {
                Kind::Interface => {
                    for m in trait_or_if.methods.values() {
                        if let Some(old) = requirements.insert(m.name.clone(), m.sig.clone()) {
                            if old != m.sig { return Err(diag(d.pos, "E_SIGNATURE", "conflicting interface method signatures")); }
                        }
                    }
                }
                Kind::Trait => for (key, method) in &trait_or_if.methods {
                    let same = inherited.entry(key.clone()).or_default();
                    if !same.iter().any(|m| m.origin == method.origin) { same.push(method.clone()); }
                },
                Kind::Class => return Err(diag(d.pos, "E_TYPE", "implements expects interface or trait, not class")),
            }
        }
        let mut result = BTreeMap::new();
        let mut inherited_concrete_names = BTreeSet::new();
        for (name, choices) in inherited {
            if let Some(local) = d.methods.get(&name) {
                if !local.overriding {
                    return Err(diag(local.pos, "E_OVERRIDE_REQUIRED",
                        format!("'{name}' must explicitly override the inherited implementation")));
                }
                // Every competing origin must have an identical signature.
                // Never allow a child to silently change parameter/return ABI
                // or narrow a public inherited method to private.
                for candidate in &choices {
                    if local.sig != candidate.sig {
                        return Err(diag(local.pos, "E_SIGNATURE",
                            format!("override '{name}' must match all inherited method signatures")));
                    }
                    if candidate.public && !local.public {
                        return Err(diag(local.pos, "E_PRIVATE",
                            format!("override '{name}' cannot reduce inherited visibility")));
                    }
                }
                inherited_concrete_names.insert(name);
            } else if choices.len() > 1 {
                return Err(diag(d.pos, "E_AMBIGUOUS_MEMBER",
                    format!("ambiguous concrete inherited method '{name}'; define an explicit override")));
            } else {
                result.insert(name, choices[0].clone());
            }
        }
        for (name, local) in &d.methods {
            // Inherited methods can originate from any ancestor depth, not
            // solely direct parent methods. An interface signature alone is
            // not a concrete method that may be overridden.
            if local.overriding && !inherited_concrete_names.contains(name) {
                return Err(diag(local.pos, "E_OVERRIDE",
                    format!("method '{name}' has no concrete inherited method to override")));
            }
            result.insert(name.clone(), local.clone());
        }
        for (name, sig) in requirements {
            let selected = result.get(&name).ok_or_else(|| diag(d.pos, "E_INTERFACE", format!("missing required interface method '{name}'")))?;
            if !selected.public { return Err(diag(selected.pos, "E_PRIVATE", format!("interface method '{name}' must be public"))); }
            if selected.sig != sig {
                return Err(diag(selected.pos, "E_SIGNATURE", format!("interface method '{name}' signature mismatch")));
            }
        }
        Ok(result)
    })();
    visiting.remove(owner);
    let answer = answer?;
    cached.insert(owner.to_owned(), answer.clone());
    Ok(answer)
}
/// Desugar supported nominal composition into ordinary class methods while
/// preserving each source token's original file position.
pub fn lower(t: Vec<Token>) -> R<Vec<Token>> {
    if !t.iter().any(|x| matches!(&x.tok, Tok::Ident(s) if s == "trait" || s == "interface" || s == "extends" || s == "implements" || s == "use")) {
        return Ok(t);
    }
    let all = decls(&t)?;
    if !all.values().any(|d| d.requires_lowering) { return Ok(t); }
    let mut cache = BTreeMap::new();
    let mut visiting = BTreeSet::new();
    let mut replace = BTreeMap::<usize, (usize, Vec<Token>)>::new();
    for d in all.values() {
        if d.kind != Kind::Class {
            replace.insert(d.start, (d.end, Vec::new()));
            continue;
        }
        if !d.requires_lowering { continue; }
        let aliases = alias_declarations(d)?;
        let physical_fields = checked_scalar_aliases(d, &all, &aliases)?;
        let resolved = resolve(&d.name, &all, &mut cache, &mut visiting)?;
        let mut additions = Vec::new();
        for (name, method) in resolved {
            if !d.methods.contains_key(&name) {
                // Prevent copied inherited methods from leaking private base
                // state via self. Inherited fields/constructors are also gated.
                if method.tokens.iter().any(|t| is(std::slice::from_ref(t), 0, "super")) {
                    return Err(diag(method.pos, "E_UNSUPPORTED", "super receiver lowering is not implemented"));
                }
                additions.extend(method.tokens);
            }
        }
        let mut output = Vec::new();
        if d.export { output.push(Token { pos: d.pos, tok: Tok::Ident("pub".into()) }); }
        output.extend([
            Token { pos: d.pos, tok: Tok::Ident("define".into()) },
            Token { pos: d.pos, tok: Tok::Ident("class".into()) },
            Token { pos: d.pos, tok: Tok::Ident(d.name.clone()) },
            Token { pos: d.pos, tok: Tok::Ident("as".into()) },
        ]);
        // Emit one parent-derived physical field for each verified alias,
        // before local state so parent initialization happens first.
        output.extend(physical_fields);
        // Never pass 'use Parent.field as alias' through to the ordinary
        // parser; its exact original slot is already represented above.
        let skipped: BTreeSet<usize> = aliases.iter()
            .flat_map(|a| a.start..a.end).collect();
        // Remove only exact declaration-header override tokens. A token
        // spelled 'override' inside a method body is never a header marker.
        // Source coordinates remain unchanged for every retained token.
        for (i, token) in d.body.iter().enumerate() {
            if skipped.contains(&i) { continue; }
            let header_override = is(std::slice::from_ref(token), 0, "override")
                && d.methods.values().any(|m|
                    m.override_pos.is_some_and(|p|
                        p.line == token.pos.line && p.col == token.pos.col));
            if !header_override { output.push(token.clone()); }
        }
        output.extend(additions);
        output.push(Token { pos: d.pos, tok: Tok::Ident("end".into()) });
        replace.insert(d.start, (d.end, output));
    }
    let mut out = Vec::with_capacity(t.len());
    let mut i = 0;
    while i < t.len() {
        if let Some((end, span)) = replace.get(&i) {
            out.extend_from_slice(span);
            i = *end;
        } else {
            out.push(t[i].clone());
            i += 1;
        }
    }
    if out.len() > crate::lexer::MAX_TOKENS {
        return Err(diag(t.first().map(|x| x.pos).unwrap_or_default(), "E_SOURCE_LIMIT",
            "expanded nominal methods exceed compiler token budget"));
    }
    Ok(out)
}

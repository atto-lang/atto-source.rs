//! Frozen registry namespace resolver and bounded direct-function guest linker.
//! Explicitly bounded profile: bounded direct source files per exported directory,
//! direct functions and scalar literal constants only, declared namespace equals manifest namespace key.
//! No source is loaded by a manifest path; inputs are immutable pinned bytes.
use crate::{Diagnostic, Pos};
use crate::import_v2::Kind;
use crate::package_resolver::{SnapshotSource, verify_registry_snapshot};
use crate::static_import_graph::{ExportView, Origin, PackageId, Scope, Target, Tier};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
type R<T> = Result<T, Diagnostic>;

fn err(code: &'static str, message: &str) -> Diagnostic {
    Diagnostic::new(Pos::default(), code, message)
}
fn component(s: &str) -> bool {
    !s.is_empty() && s.len() <= 64 && s.as_bytes()[0].is_ascii_alphanumeric()
        && s.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-'))
}
fn directory(s: &str) -> bool {
    s.len() <= 256 && s.strip_suffix('/').is_some_and(|p|
        p.split('/').all(component))
}
fn subpath(s: &str) -> bool {
    s.len() <= 256 && s.split('/').all(component)
}
fn namespace(s: &str) -> bool {
    s.split('.').count() <= 8 && s.split('.').all(|p|
        !p.is_empty() && p.len() <= 64 && p.is_ascii()
        && (p.as_bytes()[0].is_ascii_alphabetic() || p.starts_with('_'))
        && p.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
        && !p.starts_with("__atto_") && !crate::parser::is_reserved(p)
        && !matches!(p,"pkg"|"enum"|"use"|"declare"|"mtd"))
}
fn quoted(s: &str) -> R<&str> {
    let raw = s.trim().strip_prefix('"').and_then(|s| s.strip_suffix('"'))
        .ok_or_else(|| err("E_MANIFEST", "expected simple quoted string"))?;
    if raw.is_empty() || !raw.is_ascii() || raw.bytes().any(|b| b.is_ascii_control() || b == b'"' || b == b'\\') {
        return Err(err("E_MANIFEST", "unsupported manifest string"));
    }
    Ok(raw)
}
#[derive(Default)]
struct Manifest {
    package: BTreeMap<String,String>,
    namespaces: BTreeMap<String,String>,
    exports: BTreeMap<String,String>,
}
fn parse_manifest(text: &str) -> R<Manifest> {
    let mut result = Manifest::default();
    let mut table = "";
    let mut tables = BTreeSet::new();
    for line in text.lines().map(str::trim) {
        if line.is_empty() || line.starts_with('#') { continue; }
        if line.starts_with('[') {
            table = match line {
                "[package]" => "package", "[namespaces]" => "namespaces", "[exports]" => "exports",
                _ => return Err(err("E_MANIFEST", "unsupported registry manifest table")),
            };
            if !tables.insert(table) { return Err(err("E_MANIFEST", "duplicate registry manifest table")); }
            continue;
        }
        let (key,value) = line.split_once('=').ok_or_else(|| err("E_MANIFEST", "expected registry key=value"))?;
        let key = key.trim();
        let value = quoted(value)?;
        let (key, map) = match table {
            "package" if matches!(key,"name"|"version") => (key, &mut result.package),
            "namespaces" => {
                let key = if key.starts_with('"') { quoted(key)? } else { key };
                if !namespace(key) || !directory(value) { return Err(err("E_MANIFEST", "invalid namespace mapping")); }
                (key, &mut result.namespaces)
            }
            "exports" => {
                let key = quoted(key)?;
                if !subpath(key) || !directory(value) { return Err(err("E_MANIFEST", "invalid exported subpath")); }
                (key, &mut result.exports)
            }
            _ => return Err(err("E_MANIFEST", "unexpected registry manifest key")),
        };
        if map.len() >= 64 || map.insert(key.into(),value.into()).is_some() {
            return Err(err("E_MANIFEST", "duplicate or excessive registry manifest entries"));
        }
    }
    if result.package.len() != 2 || result.namespaces.is_empty() {
        return Err(err("E_MANIFEST", "registry manifest requires package identity and namespaces"));
    }
    Ok(result)
}

/// Constructible only from a complete lock-pinned source snapshot.
/// Expected hash and package selector must be independently trusted by caller.
pub struct FrozenRegistry {
    name: String,
    id: PackageId,
    namespaces: BTreeSet<String>,
    subpaths: BTreeMap<String,String>,
    view: ExportView,
    // Owned, verified source bytes; cannot be replaced after verification.
    sources: BTreeMap<String,(String,String)>,
    // Physical owner of every admitted function or literal, from pinned bytes.
    declaration_sources: BTreeMap<String,String>,
    literals: BTreeMap<String,Vec<crate::lexer::Tok>>,
    mixed_functions: BTreeMap<String,Vec<crate::lexer::Token>>,
}
impl FrozenRegistry {
    pub fn verify(expected_name: &str, manifest: &str, lock: &str, expected_lock: &str,
        inputs: &[SnapshotSource<'_>]) -> R<Self> {
        let sources = verify_registry_snapshot(manifest, lock, expected_lock, inputs)?;
        let parsed = parse_manifest(manifest)?;
        let name = parsed.package.get("name").ok_or_else(|| err("E_MANIFEST", "missing package name"))?;
        let version = parsed.package.get("version").ok_or_else(|| err("E_MANIFEST", "missing package version"))?;
        if name != expected_name { return Err(err("E_LOCK_HASH", "pinned package identity differs from requested name")); }
        let id = PackageId::new(name, version, expected_lock)?;
        let mut view = ExportView::new(Origin::External(id.clone()));
        let mut directories = BTreeMap::new();
        let mut namespaces = BTreeSet::new();
        let mut verified_sources = BTreeMap::new();
        let mut declaration_sources = BTreeMap::new();
        let mut literals = BTreeMap::new();
        let mut mixed_functions = BTreeMap::new();
        // Nested namespace parents must be declared independently. For now
        // overlapping source directories are forbidden, even if pinned.
        for (ns, dir) in &parsed.namespaces {
            if directories.keys().any(|prior: &String| prior.starts_with(dir.as_str()) || dir.starts_with(prior.as_str())) {
                return Err(err("E_MANIFEST", "overlapping registry namespace directories"));
            }
            for (at, _) in ns.match_indices('.') {
                if !parsed.namespaces.contains_key(&ns[..at]) {
                    return Err(err("E_MANIFEST", "undeclared parent namespace"));
                }
            }
            use crate::lexer::{self,Tok};
            // BTreeMap iteration fixes ordering by pinned logical path;
            // no filesystem listing or caller input order is consulted.
            let matching:Vec<_>=sources.iter().filter(|(path,_)|path.starts_with(dir.as_str())).collect();
            if matching.is_empty(){return Err(err("E_MANIFEST","namespace directory has no pinned source"));}
            if matching.len()>16 || matching.iter().any(|(path,_)|path[dir.len()..].contains('/')){
                return Err(err("E_UNSUPPORTED","namespace requires at most 16 direct pinned files"));
            }
            let header=lexer::lex(&format!("use namespace {ns};"))?;
            let end=header.len()-1;
            let mut files=Vec::new();
            let mut owners=BTreeMap::new();
            let mut total=0usize;
            for (&file,&source) in matching.iter().copied(){
                let normalized=crate::namespace::normalize(&lexer::lex(source)?,Some(Path::new(file)))?;
                if normalized.iter().any(|t|matches!(&t.tok,Tok::Ident(s) if s.starts_with("__atto_"))){
                    return Err(err("E_RESERVED","compiler-generated identifier in registry source"));
                }
                if normalized.len()<=end || normalized[..end].iter().map(|t|&t.tok).ne(header[..end].iter().map(|t|&t.tok)){
                    return Err(err("E_MANIFEST","source namespace differs from manifest key"));
                }
                total=total.saturating_add(normalized.len());
                if total>lexer::MAX_TOKENS {return Err(err("E_SOURCE_LIMIT","namespace aggregate token budget exceeded"));}
                if matching.len()>1 {
                    for (name,public) in crate::const_import_link::registry_file_inventory(&normalized)? {
                        if owners.insert(name,(file,public)).is_some(){
                            return Err(err("E_DUPLICATE","duplicate declaration across namespace files"));
                        }
                    }
                }
                files.push(normalized);
            }
            // All files in this unit belong to the same manifest namespace
            // and have already passed the exact lock and independent inventory
            // checks. Private names may be used inside this closed namespace.
            // The public ExportView still denies access to private members;
            // unpinned contextual roots do not acquire this authority.
            let (&file,&source)=matching[0];
            let source_for = |name: &str| owners.get(name).map_or(file, |(path,_)| *path);
            let normalized=if files.len()==1 {files.pop().unwrap()}else{
                let mut combined=header[..end].to_vec();
                for tokens in files {
                    combined.extend_from_slice(&tokens[end..tokens.len()-1]);
                }
                combined.push(header.last().unwrap().clone());
                combined
            };
            view.add(ns, ns, Kind::Namespace, true)?;
            if let Some(provider)=crate::const_import_link::registry_provider(&normalized,end)?{
                for (name,(public,value)) in provider.literals {
                    let path=format!("{ns}.{name}");
                    view.add(&path,&path,Kind::Const,public)?;
                    declaration_sources.insert(path.clone(),source_for(&name).to_string());
                    literals.insert(path,value);
                }
                for (name,public) in provider.functions {
                    let path=format!("{ns}.{name}");
                    view.add(&path,&path,Kind::Fnc,public)?;
                    declaration_sources.insert(path,source_for(&name).to_string());
                }
                if provider.function_tokens.len()>1 {
                    mixed_functions.insert(ns.clone(),provider.function_tokens);
                    verified_sources.insert(ns.clone(),(file.to_string(),source.to_string()));
                }
            }else{
                let functions=if matching.len()==1 {
                    let (declared,functions)=crate::import_link::registry_function_exports(source,Path::new(file))?;
                    if declared!=*ns {return Err(err("E_MANIFEST","source namespace differs from manifest key"));}
                    functions
                }else{
                    let body=normalized[end..].to_vec();
                    let (functions,_)=crate::import_link::functions(&body)?;
                    mixed_functions.insert(ns.clone(),body);
                    functions
                };
                for (function,public) in functions {
                    let path=format!("{ns}.{function}");
                    view.add(&path,&path,Kind::Fnc,public)?;
                    declaration_sources.insert(path,source_for(&function).to_string());
                }
                verified_sources.insert(ns.clone(),(file.to_string(),source.to_string()));
            }
            namespaces.insert(ns.clone());
            directories.insert(dir.clone(),ns.clone());
        }
        let mut subpaths = BTreeMap::new();
        for (sub, dir) in parsed.exports {
            let target = directories.get(&dir)
                .ok_or_else(|| err("E_MANIFEST", "subpath targets undeclared namespace directory"))?;
            subpaths.insert(sub,target.clone());
        }
        Ok(Self { name: name.clone(), id, namespaces, subpaths, view, sources: verified_sources, declaration_sources, literals, mixed_functions })
    }

    pub fn view(&self) -> &ExportView { &self.view }

    /// Physical provenance of a checked public declaration, not a private
    /// access capability. A target from another lock or with a forged kind
    /// must never be accepted, even when its declaration spelling matches.
    pub fn declaration_source(&self, target: &Target) -> R<&str> {
        if target.origin != Origin::External(self.id.clone()) {
            return Err(err("E_IMPORT_PATH","declaration origin differs from pinned package"));
        }
        let checked = self.view.lookup(&target.declaration)?;
        if &checked != target {
            return Err(err("E_IMPORT_KIND","declaration identity differs from checked export"));
        }
        self.declaration_sources.get(&target.declaration).map(String::as_str)
            .ok_or_else(||err("E_IMPORT_KIND","namespace has no single source owner"))
    }

    /// Whole-package imports use natural suffixes unless explicitly renamed.
    pub fn bind_package(&self, scope: &mut Scope, alias: Option<&str>) -> R<()> {
        scope.package(self.id.clone(), alias)
    }

    /// Select a declared namespace, or an explicitly exported subpath.
    /// No filename, URL, alternate package or implicit fallback is consulted.
    pub fn bind_namespace(&self, scope: &mut Scope, source: &str,
        selector: &str, binding: &str, explicit_alias: bool) -> R<()> {
        if !namespace(selector) || !namespace(binding) || (!explicit_alias && selector != binding) {
            return Err(err("E_IMPORT_PATH", "invalid namespace selector or binding"));
        }
        let selected = if source == self.name {
            if self.namespaces.contains(selector) {
                Some(selector.to_string())
            } else {
                let leaf = selector.rsplit('.').next().unwrap_or("");
                let mut candidates = self.namespaces.iter().filter(|n| n.rsplit('.').next() == Some(leaf));
                let first = candidates.next();
                if candidates.next().is_some() {
                    return Err(err("E_AMBIGUOUS", "ambiguous namespace selector leaf"));
                }
                if let Some(found) = first { Some(found.clone()) }
                else if selector == self.name.rsplit('/').next().unwrap_or("") { None }
                else { return Err(err("E_UNBOUND", "namespace is not exported")); }
            }
        } else {
            let suffix = source.strip_prefix(self.name.as_str()).and_then(|s| s.strip_prefix('/'))
                .ok_or_else(|| err("E_IMPORT_PATH", "source is not the pinned package"))?;
            if !subpath(suffix) { return Err(err("E_IMPORT_PATH", "noncanonical exported subpath")); }
            Some(self.subpaths.get(suffix).ok_or_else(|| err("E_UNBOUND", "subpath not exported"))?.clone())
        };
        let target = match selected {
            Some(ns) => self.view.lookup(&ns)?,
            None => Target { origin: Origin::External(self.id.clone()), declaration: String::new(), kind: Kind::Namespace },
        };
        scope.bind_verified(binding, if explicit_alias { Tier::Explicit } else { Tier::Deliberate }, target)
    }
}


/// Resolve a complete batch of source alias declarations against previously
/// bound, verified package namespaces. Both spellings share the production
/// builtin alias parser. No import acquisition or code generation occurs.
pub fn bind_registry_aliases(scope: &mut Scope, declarations: &str,
    packages: &[&FrozenRegistry]) -> R<()> {
    if declarations.len() > crate::SOURCE_CAP || packages.len() > 64 {
        return Err(err("E_SOURCE_LIMIT", "registry alias batch exceeds budget"));
    }
    let tokens = crate::lexer::lex(declarations)?;
    let mut i = 0;
    let mut normalized = Vec::new();
    while tokens.get(i).is_some_and(|t| t.tok != crate::lexer::Tok::Eof) {
        if !matches!(tokens.get(i).map(|t| &t.tok),Some(crate::lexer::Tok::Ident(s)) if s=="define")
            || !matches!(tokens.get(i+1).map(|t| &t.tok),Some(crate::lexer::Tok::Ident(s)) if s=="alias") {
            return Err(err("E_SYNTAX", "expected only define alias declarations"));
        }
        let (name,reference,_,next) = crate::library::parse_alias_declaration(&tokens,i)?;
        if normalized.len() >= 256 { return Err(err("E_SOURCE_LIMIT", "too many registry aliases")); }
        normalized.push((name,reference));
        i = next;
    }
    let pairs: Vec<_> = normalized.iter().map(|(n,r)| (n.as_str(),r.as_str())).collect();
    let views: Vec<_> = packages.iter().map(|p| p.view()).collect();
    scope.namespace_aliases(&pairs,&views)
}


/// Compile the bounded frozen-registry direct-function guest profile.
/// Only whole registry package imports, typed function/namespace selections and
/// source aliases are admitted. All source bytes are owned by verified
/// snapshots. No filesystem/network lookup, implicit fallback, or runtime
/// namespace objects are possible. The ordinary compiler still handles
/// type checking, private module lowering and Wasm emission.
pub fn compile_registry_guest(source: &str, packages: &[&FrozenRegistry]) -> R<Vec<u8>> {
    compile_registry_guest_inner(source, packages, None, None)
}

/// Compile an application root using an explicitly supplied, verified current
/// package. Public-only contextual access: claiming a namespace in source
/// cannot establish ownership of private exports. The optional logical path
/// is trusted package-relative metadata for default namespace normalization.
pub fn compile_contextual_guest(source: &str, current: &FrozenRegistry,
    packages: &[&FrozenRegistry], logical: Option<&Path>) -> R<Vec<u8>> {
    compile_registry_guest_inner(source, packages, Some(current), logical)
}

fn compile_registry_guest_inner<'a>(source: &str, packages: &[&'a FrozenRegistry],
    current: Option<&'a FrozenRegistry>, logical: Option<&Path>) -> R<Vec<u8>> {
    use crate::lexer::{self, Tok, Token};
    use crate::import_v2::{self, Source};
    fn word(t: Option<&Token>, value: &str) -> bool {
        matches!(t.map(|t| &t.tok),Some(Tok::Ident(s)) if s==value)
    }
    fn token(tok: Tok, pos: Pos) -> Token { Token { tok, pos } }
    fn id(name: &str, pos: Pos) -> Token { token(Tok::Ident(name.into()),pos) }
    fn at(mut d: Diagnostic, pos: Pos) -> Diagnostic { if d.pos.line == 0 { d.pos=pos; } d }
    fn source_name(t: Option<&Token>) -> R<&str> {
        match t.map(|x| &x.tok) {
            Some(Tok::Str(s)) => Ok(s),
            _ => Err(err("E_SYNTAX","registry source requires a quoted package name")),
        }
    }
    fn path_tokens(t: &[Token]) -> R<String> {
        if t.is_empty() || t.len()>15 || t.len()%2==0 {
            return Err(err("E_SYNTAX","invalid qualified registry path"));
        }
        let mut parts=Vec::new();
        for (i,x) in t.iter().enumerate() {
            match (&x.tok,i%2) {
                (Tok::Ident(s),0) => parts.push(s.as_str()),
                (Tok::Dot,1) => {},
                _ => return Err(err("E_SYNTAX","invalid qualified registry path")),
            }
        }
        let path=parts.join(".");
        if !namespace(&path) { return Err(err("E_IMPORT_PATH","invalid registry binding path")); }
        Ok(path)
    }
    if source.len()>crate::SOURCE_CAP || (packages.is_empty() && current.is_none()) || packages.len()+usize::from(current.is_some())>16 {
        return Err(err("E_SOURCE_LIMIT","registry guest source or package budget exceeded"));
    }
    let mut available=BTreeMap::new();
    for &p in packages {
        if available.insert(p.name.as_str(),p).is_some() {
            return Err(err("E_AMBIGUOUS","multiple pinned snapshots for one registry name"));
        }
    }
    if let Some(p)=current {
        if available.insert(p.name.as_str(),p).is_some() {return Err(err("E_AMBIGUOUS","current package must not also be supplied as an external dependency"));}
    }
    let tokens=crate::namespace::normalize(&lexer::lex(source)?,logical)?;
    for t in &tokens {
        if matches!(&t.tok,Tok::Ident(s) if s.starts_with("__atto_")) {
            return Err(Diagnostic::new(t.pos,"E_RESERVED","reserved compiler-generated identifier"));
        }
    }
    let mut scope=Scope::new();
    let mut admitted=BTreeMap::new();
    let mut i=0;
    let mut trees = Vec::new();
    let mut current_namespace=None;
    if word(tokens.first(),"use") {
        let pos=tokens[0].pos;
        let p=current.ok_or_else(||Diagnostic::new(pos,"E_IMPORT_PATH","namespace header requires explicit current package context"))?;
        let end=tokens.iter().position(|t|t.tok==Tok::Semi).ok_or_else(||Diagnostic::new(pos,"E_SYNTAX","unterminated namespace header"))?;
        let ns=path_tokens(&tokens[2..end]).map_err(|d|at(d,pos))?;
        if !p.namespaces.contains(&ns) {return Err(Diagnostic::new(pos,"E_IMPORT_PATH","source namespace is not declared in current package"));}
        current_namespace=Some(ns);
        i=end+1;
    }
    while word(tokens.get(i),"import") {
        let pos=tokens[i].pos;
        let start=i+1;
        let mut end=start;
        while tokens.get(end).is_some_and(|t| !matches!(t.tok,Tok::Semi|Tok::Eof)) { end+=1; }
        if tokens.get(end).is_none_or(|t|t.tok!=Tok::Semi) {
            return Err(Diagnostic::new(pos,"E_SYNTAX","unterminated registry import"));
        }
        if word(tokens.get(start),"pkg") && matches!(tokens.get(start+1).map(|t| &t.tok), Some(Tok::Str(_))) {
            let name=source_name(tokens.get(start+1)).map_err(|d|at(d,pos))?;
            let p=*available.get(name).ok_or_else(||Diagnostic::new(pos,"E_UNBOUND","package is not in authenticated snapshot set"))?;
            let alias=if start+2==end { None } else {
                if !word(tokens.get(start+2),"as") || start+3>=end {
                    return Err(Diagnostic::new(pos,"E_SYNTAX","expected package as qualified.alias"));
                }
                Some(path_tokens(&tokens[start+3..end]).map_err(|d|at(d,pos))?)
            };
            p.bind_package(&mut scope,alias.as_deref()).map_err(|d|at(d,pos))?;
            admitted.insert(p.name.as_str(),p);
        } else if import_v2::is_mixed(&tokens,start)
            || ((word(tokens.get(start),"const") || word(tokens.get(start),"fnc"))
                && (start+1..end).any(|j|word(tokens.get(j),"from"))) {
            let (parsed,next)=import_v2::parse_mixed(&tokens,start)?;
            if next!=end+1 { return Err(Diagnostic::new(pos,"E_SYNTAX","invalid import extent")); }
            let Source::Package(name)=&parsed.source else {
                return Err(Diagnostic::new(pos,"E_UNSUPPORTED","registry guest requires explicit pkg domain"));
            };
            let p=*available.get(name.as_str()).ok_or_else(||Diagnostic::new(pos,"E_UNBOUND","package is not in authenticated snapshot set"))?;
            if parsed.selectors.iter().any(|s| !matches!(s.kind,Kind::Fnc|Kind::Namespace|Kind::Const)) {
                return Err(Diagnostic::new(pos,"E_UNSUPPORTED","registry guest admits function, namespace and literal constant selectors only"));
            }
            scope.bind_mixed(p.view(),&parsed).map_err(|d|at(d,pos))?;
            admitted.insert(p.name.as_str(),p);
        } else if word(tokens.get(start),"namespace") && (start+1..end).any(|j| word(tokens.get(j),"from")) {
            let from=(start+1..end).find(|&j|word(tokens.get(j),"from"))
                .ok_or_else(||Diagnostic::new(pos,"E_SYNTAX","expected namespace from pkg"))?;
            if !word(tokens.get(from+1),"pkg") || from+3!=end {
                return Err(Diagnostic::new(pos,"E_SYNTAX","expected namespace from pkg quoted source"));
            }
            let name=source_name(tokens.get(from+2)).map_err(|d|at(d,pos))?;
            let root=name.split('/').take(3).collect::<Vec<_>>().join("/");
            let p=*available.get(root.as_str()).ok_or_else(||Diagnostic::new(pos,"E_UNBOUND","package is not in authenticated snapshot set"))?;
            let rename=(start+1..from).find(|&j|word(tokens.get(j),"as"));
            let selector=path_tokens(&tokens[start+1..rename.unwrap_or(from)]).map_err(|d|at(d,pos))?;
            let binding=if let Some(j)=rename { path_tokens(&tokens[j+1..from]).map_err(|d|at(d,pos))? } else { selector.clone() };
            p.bind_namespace(&mut scope,name,&selector,&binding,rename.is_some()).map_err(|d|at(d,pos))?;
            admitted.insert(p.name.as_str(),p);
        } else {
            let (leaves,next)=crate::import_tree::parse(&tokens,start)?;
            if next != end+1 { return Err(Diagnostic::new(pos,"E_SYNTAX","invalid tree extent")); }
            trees.push((leaves,pos));
        }
        i=end+1;
    }
    // All explicit package admissions precede dependent tree resolution.
    // Contextual roots never use the external alias table as an authority.
    for (leaves,pos) in trees {
        let contextual=leaves.iter().filter(|l|matches!(l.path.split('.').next(),Some("pkg"|"self"|"super"))).count();
        if contextual!=0 {
            if contextual!=leaves.len(){return Err(Diagnostic::new(pos,"E_IMPORT_PATH","mixed contextual and external tree roots"));}
            let p=current.ok_or_else(||Diagnostic::new(pos,"E_IMPORT_PATH","contextual import requires verified current package"))?;
            crate::import_tree::bind_contextual(&mut scope,&leaves,current_namespace.as_deref(),p.view()).map_err(|d|at(d,pos))?;
            admitted.insert(p.name.as_str(),p);
        }else{
            let views:Vec<_>=admitted.values().map(|p|p.view()).collect();
            crate::import_tree::bind(&mut scope,&leaves,&views).map_err(|d|at(d,pos))?;
        }
    }
    if admitted.is_empty() { return Err(err("E_UNSUPPORTED","registry guest requires an authenticated package import")); }
    let mut declarations=Vec::new();
    while word(tokens.get(i),"define") && word(tokens.get(i+1),"alias") {
        let (name,reference,pos,next)=crate::library::parse_alias_declaration(&tokens,i)?;
        if declarations.len()>=256 { return Err(Diagnostic::new(pos,"E_SOURCE_LIMIT","too many aliases")); }
        declarations.push((name,reference)); i=next;
    }
    let pairs:Vec<_>=declarations.iter().map(|(n,r)|(n.as_str(),r.as_str())).collect();
    let views:Vec<_>=admitted.values().map(|p|p.view()).collect();
    scope.namespace_aliases(&pairs,&views).map_err(|d|at(d,Pos{line:1,col:1}))?;

    // Prepare deterministic private module owners for each verified source
    // namespace. Package name order, not caller or import order, determines
    // synthetic owner names. No synthesized names can occur in source.
    let mut modules=BTreeMap::new();
    let mut linked=Vec::new();
    for p in admitted.values() {
        for (ns,(file,body)) in &p.sources {
            if modules.len()>=64 { return Err(err("E_SOURCE_LIMIT","too many linked registry namespaces")); }
            let owner=format!("__atto_registry_{}",modules.len());
            let normalized=crate::namespace::normalize(&lexer::lex(body)?,Some(Path::new(file)))?;
            let semi=normalized.iter().position(|t|t.tok==Tok::Semi).ok_or_else(||err("E_SYNTAX","missing verified namespace header"))?;
            let funcs=p.mixed_functions.get(ns).map_or(&normalized[semi+1..],Vec::as_slice);
            let (names,heads)=crate::import_link::functions(funcs)?;
            // Before joining, typecheck the source in isolation. A synthetic
            // entry is only for this check and never enters the linked guest.
            // Reject references to main and qualified accesses in dependencies;
            // no ambient root/other-package authority is admitted.
            if funcs.iter().any(|t| t.tok==Tok::Dot || word(Some(t),"main")) {
                return Err(err("E_UNSUPPORTED","registry dependency must be a closed direct-function unit"));
            }
            // Reject free calls, callable-value capture and shadowing of
            // namespace-owned routines before converting them into qualified
            // calls. Only closed direct calls survive this deliberately
            // conservative first executable lane.
            for (j,t) in funcs.iter().enumerate() {
                if heads.contains(&j) { continue; }
                if let Tok::Ident(s)=&t.tok {
                    let direct=funcs.get(j+1).is_some_and(|n|n.tok==Tok::LParen);
                    if (names.contains_key(s) && !direct)
                        || (direct && !names.contains_key(s)
                            && !matches!(s.as_str(),"if"|"while"|"match"|"for"|"return")) {
                        return Err(Diagnostic::new(t.pos,"E_UNSUPPORTED","registry dependency has a free call or non-call routine reference"));
                    }
                }
            }
            let mut isolated=funcs[..funcs.len()-1].to_vec();
            isolated.extend(lexer::lex("pub fnc main(): int { return 0; }")?);
            let parsed=crate::parser::Parser::new(isolated).parse_program()?;
            let checked=crate::check::check(&parsed)?;
            if !checked.imports.is_empty() { return Err(err("E_UNSUPPORTED","registry dependency cannot request ambient host imports")); }
            let pos=normalized[0].pos;
            linked.extend([id("define",pos),id("module",pos),id(&owner,pos),id("as",pos)]);
            for (j,t) in funcs.iter().enumerate() {
                if t.tok==Tok::Eof { continue; }
                if !heads.contains(&j) && funcs.get(j+1).is_some_and(|n|n.tok==Tok::LParen) {
                    if let Tok::Ident(s)=&t.tok {
                        if names.contains_key(s) {
                            linked.extend([id(&owner,t.pos),token(Tok::Dot,t.pos)]);
                        }
                    }
                }
                linked.push(t.clone());
            }
            linked.push(id("end",pos));
            modules.insert((p.name.clone(),ns.clone()),owner);
            if linked.len()>lexer::MAX_TOKENS { return Err(err("E_SOURCE_LIMIT","registry guest token budget exceeded")); }
        }
    }
    // Only direct static calls can consume registry bindings. Every reference
    // beginning with an admitted binding root must resolve to a public fnc.
    // Unknown qualified expressions fail closed, not as ambient members.
    while i<tokens.len() {
        let t=&tokens[i];
        if let Tok::Ident(name)=&t.tok {
            if matches!(name.as_str(),"define"|"declare"|"import"|"use"|"module"|"class"|"actor"|"enum"|"external") {
                return Err(Diagnostic::new(t.pos,"E_UNSUPPORTED","registry guest root admits direct functions only"));
            }
            if scope.has_root(name) {
                if i>0 && tokens[i-1].tok==Tok::Dot {
                    return Err(Diagnostic::new(t.pos,"E_UNSUPPORTED","registry namespace is not a runtime member"));
                }
                let start=i;
                let mut end=i+1;
                while tokens.get(end).is_some_and(|x|x.tok==Tok::Dot)
                    && matches!(tokens.get(end+1).map(|x|&x.tok),Some(Tok::Ident(_))) { end+=2; }
                let reference=path_tokens(&tokens[start..end]).map_err(|d|at(d,t.pos))?;
                let resolved=scope.resolve(&reference).map_err(|d|at(d,t.pos))?;
                let p=admitted.values().find(|p|resolved.target.origin==Origin::External(p.id.clone()))
                    .ok_or_else(||Diagnostic::new(t.pos,"E_IMPORT_PATH","resolved origin is not admitted"))?;
                let target=scope.resolve_export(&reference,p.view()).map_err(|d|at(d,t.pos))?;
                if target.kind==Kind::Const {
                    // No mutation, name declaration, borrow, invocation or
                    // dynamic namespace access. Parenthesize literal tokens
                    // to preserve expression precedence and signed i64 min.
                    let mut context=Vec::new();
                    if start>0 {context.push(tokens[start-1].clone());}
                    let context_index=context.len();
                    context.push(t.clone());
                    if let Some(next)=tokens.get(end){context.push(next.clone());}
                    if let Some(code)=crate::const_import_link::is_disallowed_reference_context(&context,context_index){
                        return Err(Diagnostic::new(t.pos,code,"constant used in disallowed context"));
                    }
                    let value=p.literals.get(&target.declaration).ok_or_else(||err("E_INTERNAL","verified literal missing"))?;
                    linked.push(token(Tok::LParen,t.pos));
                    linked.extend(value.iter().cloned().map(|v|token(v,t.pos)));
                    linked.push(token(Tok::RParen,t.pos));
                    i=end;continue;
                }
                if tokens.get(end).is_none_or(|x|x.tok!=Tok::LParen) {
                    return Err(Diagnostic::new(t.pos,"E_UNSUPPORTED","registry binding supports direct calls only"));
                }
                if target.kind!=Kind::Fnc { return Err(Diagnostic::new(t.pos,"E_IMPORT_KIND","registry call requires exported function")); }
                let (ns,member)=target.declaration.rsplit_once('.').ok_or_else(||err("E_INTERNAL","invalid verified function identity"))?;
                let owner=modules.get(&(p.name.clone(),ns.to_string())).ok_or_else(||err("E_INTERNAL","verified function owner not linked"))?;
                linked.extend([id(owner,t.pos),token(Tok::Dot,t.pos),id(member,t.pos)]);
                i=end; continue;
            }
        }
        if t.tok==Tok::Dot {
            return Err(Diagnostic::new(t.pos,"E_UNBOUND","unbound qualified registry guest reference"));
        }
        linked.push(t.clone()); i+=1;
        if linked.len()>lexer::MAX_TOKENS { return Err(err("E_SOURCE_LIMIT","registry guest token budget exceeded")); }
    }
    crate::compile_prelexed(linked,None)
}


/// First executable nominal import lane. Deliberately restricted to one
/// lock-pinned class file in one namespace, with no other package sources.
/// The general registry linker continues to reject nominal provider units.
pub struct FrozenClass {
    name: String,
    view: ExportView,
    declaration: String,
    class_name: String,
    sealed_factory: bool,
    tokens: Vec<crate::lexer::Token>,
}

impl FrozenClass {
    pub fn verify(expected_name: &str, manifest: &str, lock: &str, expected_lock: &str,
        inputs: &[SnapshotSource<'_>]) -> R<Self> {
        Self::verify_inner(expected_name,manifest,lock,expected_lock,inputs,false)
    }

    /// Explicit canonical factory lane; unlike the older preview, requires
    /// a compiler-owned, exact self-target Newable witness.
    pub fn verify_newable(expected_name: &str, manifest: &str, lock: &str, expected_lock: &str,
        inputs: &[SnapshotSource<'_>]) -> R<Self> {
        Self::verify_inner(expected_name,manifest,lock,expected_lock,inputs,true)
    }

    fn verify_inner(expected_name: &str, manifest: &str, lock: &str, expected_lock: &str,
        inputs: &[SnapshotSource<'_>], sealed_factory: bool) -> R<Self> {
        let (name,id,ns,tokens)=verified_class_tokens(expected_name,manifest,lock,expected_lock,inputs)?;
        Self::admit_tokens(name,id,&ns,tokens,sealed_factory)
    }

    fn admit_tokens(name: String, id: PackageId, ns: &str, mut tokens: Vec<crate::lexer::Token>,
        sealed_factory: bool) -> R<Self> {
        use crate::lexer::{self, Tok};
        // No ambient source names, host imports, other declarations or
        // package-only field privileges can leak into a merged guest.
        for (i,t) in tokens.iter().enumerate() {
            if let Tok::Ident(n)=&t.tok {
                if n.starts_with("__atto_") || matches!(n.as_str(),"main"|"import"|"external"|"pkg"|"use"|"declare") {
                    return Err(err("E_UNSUPPORTED","class provider requires a closed source-local class"));
                }
                // The current AST does not retain constructor visibility.
                // Require explicit public constructor syntax, never infer it.
                if n=="constructor" && !i.checked_sub(1).and_then(|j|tokens.get(j))
                    .is_some_and(|t|matches!(&t.tok,Tok::Ident(v) if v=="pub")) {
                    return Err(err("E_PRIVATE","external constructor must be explicitly public"));
                }
            }
        }
        if sealed_factory {
            tokens=lower_sealed_factory(tokens)?;
        } else if tokens.iter().any(|t|matches!(&t.tok,Tok::Ident(n) if n=="Newable" || n=="new")) {
            return Err(err("E_UNSUPPORTED","Newable factories require the sealed factory admission API"));
        }
        let program=crate::parser::Parser::new(tokens.clone()).parse_program()?;
        if program.classes.len()!=1 || !program.funcs.is_empty() || !program.enums.is_empty()
            || !program.actors.is_empty() || program.actor_interface.is_some() {
            return Err(err("E_UNSUPPORTED","class provider must contain exactly one class"));
        }
        let class=&program.classes[0];
        if !class.public || class.state_container || (!sealed_factory && class.ctor.is_none())
            || class.fields.iter().any(|f|f.visibility==crate::ast::FieldVisibility::Package) {
            return Err(err("E_UNSUPPORTED","class provider requires a public ordinary class with explicit constructor"));
        }
        if sealed_factory {
            let factory=class.methods.iter().find(|m|m.name==SEALED_FACTORY)
                .ok_or_else(||err("E_NEWABLE","sealed factory is not a class member"))?;
            let result=match factory.ret {
                crate::ast::Type::Named(i)=>program.type_names.get(i as usize).map(|(n,_)|n.as_str()),
                _=>None,
            };
            if !factory.public || !factory.is_static || !factory.params.is_empty()
                || result!=Some(class.name.as_str()) || class.ctor.as_ref().is_some_and(|c|!c.params.is_empty()) {
                return Err(err("E_NEWABLE","factory must be public static zero-arg and return its exact class"));
            }
        }
        let mut isolated=tokens[..tokens.len()-1].to_vec();
        isolated.extend(lexer::lex("pub fnc main(): int { return 0; }")?);
        let program=crate::parser::Parser::new(isolated).parse_program()?;
        let checked=crate::check::check(&program)?;
        if !checked.imports.is_empty() {
            return Err(err("E_UNSUPPORTED","class provider cannot request ambient host imports"));
        }
        let class_name=class.name.clone();
        let declaration=format!("{ns}.{class_name}");
        let mut view=ExportView::new(Origin::External(id));
        view.add(ns,ns,Kind::Namespace,true)?;
        view.add(&declaration,&declaration,Kind::Class,true)?;
        Ok(Self{name,view,declaration,class_name,sealed_factory,tokens})
    }

    /// Explicit class selection only; aliases and multi-class layout
    /// remapping are intentionally unsupported in this first guest lane.
    pub fn compile_guest(&self, source: &str) -> R<Vec<u8>> {
        use crate::lexer::{self, Tok};
        use crate::import_v2::{self, Source};
        if source.len()>crate::SOURCE_CAP {return Err(err("E_SOURCE_LIMIT","class guest source limit"));}
        let tokens=lexer::lex(source)?;
        if !matches!(tokens.first().map(|t|&t.tok),Some(Tok::Ident(n)) if n=="import") {
            return Err(err("E_SYNTAX","class guest requires an explicit typed import"));
        }
        let (import,end)=import_v2::parse_mixed(&tokens,1)?;
        if import.source!=Source::Package(self.name.clone()) {
            return Err(err("E_IMPORT_PATH","class import differs from pinned package"));
        }
        if import.selectors.len()!=1 {return Err(err("E_UNSUPPORTED","single-class guest requires one selection"));}
        let selection=&import.selectors[0];
        if selection.kind!=Kind::Class {return Err(err("E_IMPORT_KIND","class import requires class selector"));}
        if selection.path!=self.declaration {return Err(err("E_UNBOUND","unknown pinned class"));}
        if selection.binding!=self.class_name || selection.explicit_alias {
            return Err(err("E_UNSUPPORTED","class aliases require nominal remapping"));
        }
        let mut scope=Scope::new();
        scope.bind_mixed(&self.view,&import)?;
        let _identity=scope.resolve_nominal(&self.class_name,&[&self.view])?;
        let mut root=tokens[end..].to_vec();
        if root.iter().any(|t|matches!(&t.tok,Tok::Ident(n) if n.starts_with("__atto_") || matches!(n.as_str(),"import"|"use"|"define"|"declare"|"external"))) {
            return Err(err("E_UNSUPPORTED","class guest root must be a closed entry function"));
        }
        if self.sealed_factory { lower_factory_calls(&mut root,&self.class_name)?; }
        let parsed=crate::parser::Parser::new(root.to_vec()).parse_program()?;
        if parsed.funcs.len()!=1 || parsed.funcs[0].name!="main" || !parsed.classes.is_empty()
            || !parsed.enums.is_empty() || !parsed.actors.is_empty() || parsed.actor_interface.is_some() {
            return Err(err("E_UNSUPPORTED","class guest root must contain only main"));
        }
        let mut linked=self.tokens[..self.tokens.len()-1].to_vec();
        linked.extend_from_slice(&root);
        if linked.len()>lexer::MAX_TOKENS {return Err(err("E_SOURCE_LIMIT","class guest token budget exceeded"));}
        // Parse and typecheck the combined declaration with the real checker.
        // In particular, constructor arity, private member access and field
        // ownership are checked before the ordinary emission pipeline.
        let parsed=crate::parser::Parser::new(linked.clone()).parse_program()?;
        let checked=crate::check::check(&parsed)?;
        if !checked.imports.is_empty() {return Err(err("E_UNSUPPORTED","class guest cannot request ambient host imports"));}
        crate::compile_prelexed(linked,None)
    }
}


/// Link a bounded set of independently verified single-class packages.
/// Each admitted snapshot receives exactly one deterministic internal class
/// name. Explicit import aliases are not themselves nominal identities.
/// This lane permits constructor, scalar static calls and colon-annotated
/// type references; arbitrary class-name use or provider self-type references
/// must wait for AST-level remapping.
pub fn compile_frozen_classes(source: &str, packages: &[&FrozenClass]) -> R<Vec<u8>> {
    use crate::lexer::{self, Tok};
    use crate::import_v2::{self, Source};
    if source.len()>crate::SOURCE_CAP || packages.is_empty() || packages.len()>16 {
        return Err(err("E_SOURCE_LIMIT","bounded class guest input limit"));
    }
    let mut available=BTreeMap::new();
    for p in packages {
        if available.insert(p.name.as_str(),*p).is_some() {
            return Err(err("E_DUPLICATE","multiple snapshots for one class package"));
        }
    }
    let tokens=lexer::lex(source)?;
    let word=|i:usize,s:&str|matches!(tokens.get(i).map(|t|&t.tok),Some(Tok::Ident(n)) if n==s);
    let mut scope=Scope::new();
    let mut selected=BTreeMap::new();
    let mut aliases=BTreeMap::new();
    let mut cursor=0;
    while word(cursor,"import") {
        let (import,end)=import_v2::parse_mixed(&tokens,cursor+1)?;
        let Source::Package(name)=&import.source else {return Err(err("E_IMPORT_PATH","class import requires a pinned package"));};
        let p=*available.get(name.as_str()).ok_or_else(||err("E_UNBOUND","class package not supplied"))?;
        if import.selectors.len()!=1 {return Err(err("E_UNSUPPORTED","one class selector per import"));}
        let sel=&import.selectors[0];
        if sel.kind!=Kind::Class {return Err(err("E_IMPORT_KIND","class selector required"));}
        if sel.path!=p.declaration {return Err(err("E_UNBOUND","class not in pinned package"));}
        if sel.binding=="main" || aliases.contains_key(&sel.binding) {
            return Err(err("E_DUPLICATE","duplicate or reserved class binding"));
        }
        scope.bind_mixed(&p.view,&import)?;
        let identity=scope.resolve_nominal(&sel.binding,&[&p.view])?;
        selected.insert(name.clone(),p);
        aliases.insert(sel.binding.clone(),(name.clone(),identity));
        if aliases.len()>32 {return Err(err("E_SOURCE_LIMIT","class binding limit"));}
        cursor=end;
    }
    if selected.is_empty() {return Err(err("E_SYNTAX","class guest requires explicit imports"));}
    let sealed=selected.values().all(|p|p.sealed_factory);
    if !sealed && selected.values().any(|p|p.sealed_factory) {
        return Err(err("E_NEWABLE","cannot mix sealed factory and experimental raw constructor lanes"));
    }
    let mut owners=BTreeMap::new();
    let mut linked=Vec::new();
    for (name,p) in &selected {
        let owner=format!("__atto_frozen_class_{}",owners.len());
        // Only the exact already-checked compiler-generated factory header
        // and self-allocation body may refer to the provider class name.
        // No generic textual replacement of member names or local bindings.
        let mut self_positions=BTreeSet::new();
        self_positions.insert(3usize);
        if sealed {
            let pattern=lexer::lex(&format!("pub static fnc {SEALED_FACTORY}(): {} {{ return new {}(); }}",p.class_name,p.class_name))?;
            let pattern=&pattern[..pattern.len()-1];
            let matches:Vec<_>=p.tokens.windows(pattern.len()).enumerate()
                .filter(|(_,v)|v.iter().map(|t|&t.tok).eq(pattern.iter().map(|t|&t.tok)))
                .map(|(i,_)|i).collect();
            if matches.len()!=1 {return Err(err("E_NEWABLE","missing unique checked factory body"));}
            for (i,t) in pattern.iter().enumerate() {
                if matches!(&t.tok,Tok::Ident(n) if n==&p.class_name) {
                    self_positions.insert(matches[0]+i);
                }
            }
        }
        // Provider isolation was checked before linking. Do not rewrite
        // arbitrary identifier occurrences (fields, locals, type parameters).
        // Only an exact class declaration header may contain this spelling.
        for (i,t) in p.tokens.iter().enumerate().take(p.tokens.len()-1) {
            let mut t=t.clone();
            if matches!(&t.tok,Tok::Ident(n) if n==&p.class_name) {
                if !self_positions.contains(&i) {return Err(err("E_UNSUPPORTED","class self-type/member spelling requires typed remapping"));}
                t.tok=Tok::Ident(owner.clone());
            }
            linked.push(t);
        }
        owners.insert(name.clone(),owner);
        if linked.len()>lexer::MAX_TOKENS {return Err(err("E_SOURCE_LIMIT","class declaration token budget"));}
    }
    let mut root=tokens[cursor..].to_vec();
    // Validate all original tokens before introducing the internal factory
    // name. Only an exact selected alias .new() can obtain this spelling.
    if root.iter().any(|t|matches!(&t.tok,Tok::Ident(n) if n.starts_with("__atto_") || matches!(n.as_str(),"import"|"use"|"define"|"declare"|"external"))) {
        return Err(err("E_UNSUPPORTED","class root cannot introduce ambient declarations or internal names"));
    }
    if sealed {
        for i in 0..root.len() {
            if !matches!(&root[i].tok,Tok::Ident(n) if n=="new") {continue;}
            let alias=i.checked_sub(2).and_then(|j|root.get(j)).and_then(|t|match &t.tok {
                Tok::Ident(n) if aliases.contains_key(n)=>Some(n), _=>None,
            });
            if alias.is_none() || root[i-1].tok!=Tok::Dot
                || i.checked_sub(3).and_then(|j|root.get(j)).is_some_and(|t|t.tok==Tok::Dot)
                || root.get(i+1).is_none_or(|t|t.tok!=Tok::LParen)
                || root.get(i+2).is_none_or(|t|t.tok!=Tok::RParen) {
                return Err(err("E_NEWABLE","allocation requires exact imported alias.new()"));
            }
            root[i].tok=Tok::Ident(SEALED_FACTORY.into());
        }
    }
    // Validate root structure before joining providers; no root class or
    // extra routine can capture an authenticated declaration.
    let parsed=crate::parser::Parser::new(root.clone()).parse_program()?;
    if parsed.funcs.len()!=1 || parsed.funcs[0].name!="main" || !parsed.classes.is_empty()
        || !parsed.enums.is_empty() || !parsed.actors.is_empty() || parsed.actor_interface.is_some() {
        return Err(err("E_UNSUPPORTED","class guest root must contain only main"));
    }
    for (i,t) in root.iter().enumerate() {
        let mut t=t.clone();
        if let Tok::Ident(n)=&t.tok {
            if let Some((name,_identity))=aliases.get(n) {
                let before=i.checked_sub(1).and_then(|j|root.get(j));
                let after=root.get(i+1);
                let constructor=before.is_some_and(|v|matches!(&v.tok,Tok::Ident(s) if s=="new"));
                let typed=before.is_some_and(|v|v.tok==Tok::Colon);
                let static_call=after.is_some_and(|v|v.tok==Tok::Dot)
                    && matches!(root.get(i+2).map(|v|&v.tok),Some(Tok::Ident(_)))
                    && root.get(i+3).is_some_and(|v|v.tok==Tok::LParen)
                    && !before.is_some_and(|v|v.tok==Tok::Dot);
                if !(constructor || typed || static_call) {
                    return Err(err("E_IMPORT_KIND","class binding is not a runtime value or local name"));
                }
                t.tok=Tok::Ident(owners.get(name).ok_or_else(||err("E_INTERNAL","missing class owner"))?.clone());
            }
        }
        linked.push(t);
        if linked.len()>lexer::MAX_TOKENS {return Err(err("E_SOURCE_LIMIT","class guest token budget"));}
    }
    let parsed=crate::parser::Parser::new(linked.clone()).parse_program()?;
    let checked=crate::check::check(&parsed)?;
    if !checked.imports.is_empty() {return Err(err("E_UNSUPPORTED","class guest cannot request ambient host imports"));}
    crate::compile_prelexed(linked,None)
}


const SEALED_FACTORY: &str = "__atto_sealed_newable_factory";

/// Recognize only the exact canonical self-allocation factory body in v1.
/// No lexical Newable name supplied by another source can create a witness.
/// All other uses of the reserved witness/factory words reject; the parser
/// and checker subsequently validate the lowered method and class.
fn lower_sealed_factory(mut tokens: Vec<crate::lexer::Token>) -> R<Vec<crate::lexer::Token>> {
    use crate::lexer::{self, Tok};
    let class=match tokens.get(3).map(|t|&t.tok) {
        Some(Tok::Ident(n)) if n!="Newable" => n.clone(),
        _=>return Err(err("E_NEWABLE","missing self class identity")),
    };
    let expected=lexer::lex(&format!("implements Newable[{class}] as"))?;
    let header=&expected[..expected.len()-1];
    if tokens.get(4..4+header.len()).is_none_or(|s|s.iter().map(|t|&t.tok).ne(header.iter().map(|t|&t.tok))) {
        return Err(err("E_NEWABLE","requires exact implements Newable[SelfClass]"));
    }
    if tokens.iter().filter(|t|matches!(&t.tok,Tok::Ident(n) if n=="Newable")).count()!=1 {
        return Err(err("E_NEWABLE","Newable witness cannot be redefined or referenced as a value"));
    }
    tokens.drain(4..4+header.len()-1);
    let mut matches=Vec::new();
    for spelling in ["pub fnc","pub static fnc"] {
        for delimiter in [":","->"] {
            let pattern=lexer::lex(&format!("{spelling} new() {delimiter} {class} {{ return new {class}(); }}"))?;
            let pattern=&pattern[..pattern.len()-1];
            for (i,window) in tokens.windows(pattern.len()).enumerate() {
                if window.iter().map(|t|&t.tok).eq(pattern.iter().map(|t|&t.tok)) {
                    matches.push((i,pattern.len()));
                }
            }
        }
    }
    if matches.len()!=1 || tokens.iter().filter(|t|matches!(&t.tok,Tok::Ident(n) if n=="new")).count()!=2 {
        return Err(err("E_NEWABLE","requires exactly one public zero-arg self-returning allocation factory"));
    }
    let (i,len)=matches[0];
    let replacement=lexer::lex(&format!("pub static fnc {SEALED_FACTORY}(): {class} {{ return new {class}(); }}"))?;
    tokens.splice(i..i+len,replacement.into_iter().filter(|t|t.tok!=Tok::Eof));
    Ok(tokens)
}

fn lower_factory_calls(tokens: &mut [crate::lexer::Token], class: &str) -> R<()> {
    use crate::lexer::Tok;
    for i in 0..tokens.len() {
        if !matches!(&tokens[i].tok,Tok::Ident(n) if n=="new") {continue;}
        if i<2 || !matches!(&tokens[i-2].tok,Tok::Ident(n) if n==class)
            || tokens[i-1].tok!=Tok::Dot
            || tokens.get(i+1).is_none_or(|t|t.tok!=Tok::LParen)
            || tokens.get(i+2).is_none_or(|t|t.tok!=Tok::RParen) {
            return Err(err("E_NEWABLE","external allocation must use the sealed zero-arg class factory"));
        }
        tokens[i].tok=Tok::Ident(SEALED_FACTORY.into());
    }
    Ok(())
}


/// Read the exact authenticated namespace and declaration tokens. Both
/// ordinary and generic lanes use the original trusted lock, never a
/// manufactured lock over compiler-generated specialization bytes.
fn verified_class_tokens(expected_name: &str, manifest: &str, lock: &str, expected_lock: &str,
    inputs: &[SnapshotSource<'_>]) -> R<(String,PackageId,String,Vec<crate::lexer::Token>)> {
        use crate::lexer::{self, Tok};
        let sources=verify_registry_snapshot(manifest,lock,expected_lock,inputs)?;
        let parsed=parse_manifest(manifest)?;
        let name=parsed.package.get("name").ok_or_else(||err("E_MANIFEST","missing package name"))?;
        let version=parsed.package.get("version").ok_or_else(||err("E_MANIFEST","missing version"))?;
        if name!=expected_name {return Err(err("E_LOCK_HASH","pinned package identity differs from requested name"));}
        let id=PackageId::new(name,version,expected_lock)?;
        if sources.len()!=1 || parsed.namespaces.len()!=1 || !parsed.exports.is_empty() {
            return Err(err("E_UNSUPPORTED","single-class profile requires exactly one namespace and source file"));
        }
        let (ns,dir)=parsed.namespaces.iter().next().unwrap();
        let (&file,&source)=sources.iter().next().unwrap();
        if ns.contains('.') || !file.starts_with(dir.as_str()) || file[dir.len()..].contains('/') {
            return Err(err("E_MANIFEST","class source must be directly inside its declared namespace"));
        }
        let normalized=crate::namespace::normalize(&lexer::lex(source)?,Some(Path::new(file)))?;
        let header=lexer::lex(&format!("use namespace {ns};"))?;
        let end=header.len()-1;
        if normalized.len()<=end || normalized[..end].iter().map(|t|&t.tok).ne(header[..end].iter().map(|t|&t.tok)) {
            return Err(err("E_MANIFEST","class namespace differs from manifest key"));
        }
        let mut tokens=normalized[end..].to_vec();
        let word=|i:usize,s:&str| matches!(tokens.get(i).map(|t|&t.tok),Some(Tok::Ident(n)) if n==s);
        let canonical=word(0,"define") && word(1,"pub") && word(2,"class");
        if !canonical && !(word(0,"pub") && word(1,"define") && word(2,"class")) {
            return Err(err("E_PRIVATE","single-class profile requires an explicitly public class"));
        }
        if canonical {tokens.swap(0,1);}

    Ok((name.clone(),id,ns.clone(),tokens))
}

/// One source-derived, unconstrained generic class template. Only scalar
/// method signatures and the exact sealed self-target factory are admitted.
/// No generic fields, nested generic arguments or general specialization.
pub struct FrozenGenericClass {
    name: String,
    id: PackageId,
    namespace: String,
    declaration: String,
    class_name: String,
    parameter: String,
    members: Vec<crate::lexer::Token>,
    view: ExportView,
}

impl FrozenGenericClass {
    pub fn verify(expected_name: &str, manifest: &str, lock: &str, expected_lock: &str,
        inputs: &[SnapshotSource<'_>]) -> R<Self> {
        use crate::lexer::{self,Tok};
        let (name,id,namespace,tokens)=verified_class_tokens(expected_name,manifest,lock,expected_lock,inputs)?;
        let ident=|i:usize|match tokens.get(i).map(|t|&t.tok) {
            Some(Tok::Ident(n)) if !crate::parser::is_reserved(n) && !n.starts_with("__atto_")=>Ok(n.clone()),
            _=>Err(err("E_TYPE","generic class requires an ordinary identifier")),
        };
        let class_name=ident(3)?;
        let parameter=ident(5)?;
        if class_name==parameter || matches!(parameter.as_str(),"int"|"bool"|"float"|"string"|"Newable") || class_name=="Newable" {
            return Err(err("E_TYPE","generic parameter conflicts with class or builtin"));
        }
        let header=lexer::lex(&format!("pub define class {class_name}[{parameter}] implements Newable[{class_name}[{parameter}]] as"))?;
        let header=&header[..header.len()-1];
        if tokens.get(..header.len()).is_none_or(|v|v.iter().map(|t|&t.tok).ne(header.iter().map(|t|&t.tok))) {
            return Err(err("E_NEWABLE","generic class needs exactly one parameter and exact self-target witness"));
        }
        let factory=lexer::lex(&format!("pub fnc new(): {class_name}[{parameter}] {{ return new {class_name}[{parameter}](); }}"))?;
        let factory=&factory[..factory.len()-1];
        let tail=&tokens[header.len()..];
        // Restrict the first factory to the leading exact declaration.
        if tail.get(..factory.len()).is_none_or(|v|v.iter().map(|t|&t.tok).ne(factory.iter().map(|t|&t.tok))) {
            return Err(err("E_NEWABLE","generic class requires exact leading public zero-arg factory"));
        }
        let members=tail[factory.len()..].to_vec();
        let mut braces=0usize;
        let mut parens=0usize;
        let mut signature=false;
        for (i,t) in members.iter().enumerate() {
            if let Tok::Ident(n)=&t.tok {
                if n.starts_with("__atto_") || n==&class_name || matches!(n.as_str(),"Newable"|"new"|"constructor"|"define"|"declare"|"import"|"use"|"external"|"pkg"|"main") {
                    return Err(err("E_UNSUPPORTED","generic class has unsupported member or ambient identifier"));
                }
                // Only an explicit top-level fnc header can establish a
                // generic substitution site. Body calls, field initializers
                // and nested parentheses must not manufacture type authority.
                if braces==0 && parens==0 && n=="fnc" {signature=true;}
                if n==&parameter {
                    let before=i.checked_sub(1).and_then(|j|members.get(j));
                    let before2=i.checked_sub(2).and_then(|j|members.get(j));
                    let after=members.get(i+1);
                    let parameter_type=braces==0 && signature && parens==1
                        && before.is_some_and(|v|matches!(v.tok,Tok::LParen|Tok::Comma))
                        && matches!(after.map(|v|&v.tok),Some(Tok::Ident(_)))
                        && members.get(i+2).is_some_and(|v|matches!(v.tok,Tok::Comma|Tok::RParen));
                    let result_type=braces==0 && signature && parens==0
                        && before.is_some_and(|v|matches!(v.tok,Tok::Colon|Tok::Arrow))
                        && before2.is_some_and(|v|v.tok==Tok::RParen)
                        && after.is_some_and(|v|v.tok==Tok::LBrace);
                    if !(parameter_type || result_type) {
                        return Err(err("E_UNSUPPORTED","generic parameter permitted only in top-level scalar fnc signature; no implicit generic field default"));
                    }
                }
            }
            match t.tok {
                Tok::LBrace => {braces+=1; signature=false;}
                Tok::RBrace => {braces=braces.checked_sub(1).ok_or_else(||err("E_SYNTAX","unmatched generic member brace"))?;}
                Tok::LParen => parens+=1,
                Tok::RParen => {parens=parens.checked_sub(1).ok_or_else(||err("E_SYNTAX","unmatched generic member parenthesis"))?;}
                Tok::Semi if braces==0 => signature=false,
                _=>{}
            }
            if braces>128 || parens>128 {return Err(err("E_SOURCE_LIMIT","generic member nesting budget"));}
        }
        if braces!=0 || parens!=0 {return Err(err("E_SYNTAX","unclosed generic member delimiter"));}
        let declaration=format!("{namespace}.{class_name}");
        let mut view=ExportView::new(Origin::External(id.clone()));
        view.add(&namespace,&namespace,Kind::Namespace,true)?;
        view.add_generic(&declaration,&declaration,Kind::Class,true,1)?;
        let result=Self{name,id,namespace,declaration,class_name,parameter,members,view};
        // Check every admitted scalar specialization, including unused
        // methods, before publishing the frozen template.
        for scalar in [crate::static_import_graph::ScalarIdentity::Int,crate::static_import_graph::ScalarIdentity::Bool,crate::static_import_graph::ScalarIdentity::String] {
            result.specialize(scalar)?;
        }
        Ok(result)
    }

    pub fn view(&self) -> &ExportView { &self.view }

    fn specialize(&self, scalar: crate::static_import_graph::ScalarIdentity) -> R<FrozenClass> {
        use crate::lexer::{self,Tok};
        use crate::static_import_graph::ScalarIdentity;
        let ty=match scalar {
            ScalarIdentity::Int=>"int", ScalarIdentity::Bool=>"bool", ScalarIdentity::String=>"string",
            _=>return Err(err("E_UNSUPPORTED","scalar specialization not admitted")),
        };
        let class=&self.class_name;
        let mut tokens=lexer::lex(&format!("pub define class {class} implements Newable[{class}] as pub fnc new(): {class} {{ return new {class}(); }}"))?;
        tokens.pop();
        for t in &self.members {
            let mut t=t.clone();
            if matches!(&t.tok,Tok::Ident(n) if n==&self.parameter) {
                t.tok=Tok::Ident(ty.into());
            }
            tokens.push(t);
        }
        // The original snapshot identity is retained, but this intermediate
        // monomorphic declaration is private to the specialization pipeline.
        FrozenClass::admit_tokens(self.name.clone(),self.id.clone(),&self.namespace,tokens,true)
    }

    /// Compile one explicit scalar instance per guest. Multiple instances in
    /// the same executable need a separate typed specialization registry.
    pub fn compile_guest(&self, source: &str, scalar: crate::static_import_graph::ScalarIdentity) -> R<Vec<u8>> {
        use crate::lexer::{self,Tok};
        use crate::import_v2::{self,Source};
        use crate::static_import_graph::{ScalarIdentity,TypeTerm};
        if source.len()>crate::SOURCE_CAP {return Err(err("E_SOURCE_LIMIT","generic guest source limit"));}
        let ty=match scalar {
            ScalarIdentity::Int=>"int",ScalarIdentity::Bool=>"bool",ScalarIdentity::String=>"string",
            _=>return Err(err("E_UNSUPPORTED","scalar specialization not admitted")),
        };
        let tokens=lexer::lex(source)?;
        if !matches!(tokens.first().map(|t|&t.tok),Some(Tok::Ident(n)) if n=="import") {
            return Err(err("E_SYNTAX","generic guest requires explicit class import"));
        }
        let (import,end)=import_v2::parse_mixed(&tokens,1)?;
        if import.source!=Source::Package(self.name.clone()) || import.selectors.len()!=1 {
            return Err(err("E_IMPORT_PATH","generic guest requires one pinned class selection"));
        }
        let sel=&import.selectors[0];
        if sel.kind!=Kind::Class || sel.path!=self.declaration || sel.binding!=self.class_name || sel.explicit_alias {
            return Err(err("E_IMPORT_KIND","generic guest requires original class binding"));
        }
        let mut scope=Scope::new();
        scope.bind_mixed(&self.view,&import)?;
        let _identity=scope.resolve_type_identity(&[
            TypeTerm::Scalar(scalar),TypeTerm::Nominal{reference:&self.class_name,arguments:1}
        ],&[&self.view])?;
        let root=&tokens[end..];
        let mut lowered=Vec::new();
        let mut i=0;
        while i<root.len() {
            let t=&root[i];
            if let Tok::Ident(n)=&t.tok {
                if n.starts_with("__atto_") || matches!(n.as_str(),"import"|"define"|"declare"|"use"|"external") {
                    return Err(err("E_UNSUPPORTED","generic root cannot introduce ambient declarations"));
                }
                if n==&self.class_name {
                    if root.get(i+1).is_none_or(|t|t.tok!=Tok::LBracket)
                        || !matches!(root.get(i+2).map(|t|&t.tok),Some(Tok::Ident(n)) if n==ty)
                        || root.get(i+3).is_none_or(|t|t.tok!=Tok::RBracket) {
                        return Err(err("E_TYPE","class reference must carry the exact scalar type argument"));
                    }
                    let typed=i.checked_sub(1).and_then(|j|root.get(j)).is_some_and(|t|t.tok==Tok::Colon);
                    let factory=root.get(i+4).is_some_and(|t|t.tok==Tok::Dot)
                        && matches!(root.get(i+5).map(|t|&t.tok),Some(Tok::Ident(n)) if n=="new")
                        && root.get(i+6).is_some_and(|t|t.tok==Tok::LParen)
                        && root.get(i+7).is_some_and(|t|t.tok==Tok::RParen);
                    if !typed && !factory {return Err(err("E_NEWABLE","generic class only admits explicit type or factory reference"));}
                    lowered.push(t.clone());
                    i+=4;
                    continue;
                }
            }
            lowered.push(t.clone());
            i+=1;
        }
        let instance=self.specialize(scalar)?;
        lower_factory_calls(&mut lowered,&self.class_name)?;
        let parsed=crate::parser::Parser::new(lowered.clone()).parse_program()?;
        if parsed.funcs.len()!=1 || parsed.funcs[0].name!="main" || !parsed.classes.is_empty()
            || !parsed.enums.is_empty() || !parsed.actors.is_empty() || parsed.actor_interface.is_some() {
            return Err(err("E_UNSUPPORTED","generic guest requires only main"));
        }
        let mut linked=instance.tokens[..instance.tokens.len()-1].to_vec();
        linked.extend(lowered);
        if linked.len()>lexer::MAX_TOKENS {return Err(err("E_SOURCE_LIMIT","generic guest token budget"));}
        let parsed=crate::parser::Parser::new(linked.clone()).parse_program()?;
        let checked=crate::check::check(&parsed)?;
        if !checked.imports.is_empty() {return Err(err("E_UNSUPPORTED","generic guest cannot request ambient host imports"));}
        crate::compile_prelexed(linked,None)
    }
}


impl FrozenGenericClass {
    /// Instantiate the same authenticated declaration at multiple scalar
    /// arguments in one executable. Each canonical specialization receives
    /// one private class slot, independent of encounter order or alias.
    pub fn compile_guest_instances(&self, source: &str) -> R<Vec<u8>> {
        compile_frozen_generic_packages(source,&[self])
    }
}

/// Multiple independently authenticated generic templates. The package
/// snapshot and ordered scalar argument identity select one private slot.
/// No package snapshot, import alias or scalar argument may capture another.
pub fn compile_frozen_generic_packages(source: &str, packages: &[&FrozenGenericClass]) -> R<Vec<u8>> {
        use crate::lexer::{self,Tok};
        use crate::import_v2::{self,Source};
        use crate::static_import_graph::{ScalarIdentity as S,TypeIdentity,TypeTerm};
        if source.len()>crate::SOURCE_CAP || packages.is_empty() || packages.len()>16 {
            return Err(err("E_SOURCE_LIMIT","generic guest input budget"));
        }
        let mut available=BTreeMap::new();
        for p in packages {
            if available.insert(p.name.as_str(),*p).is_some() {
                return Err(err("E_DUPLICATE","multiple snapshots for one generic package"));
            }
        }
        let tokens=lexer::lex(source)?;
        let mut cursor=0;
        let mut scope=Scope::new();
        let mut bindings=BTreeMap::new();
        let mut selected=BTreeMap::new();
        while matches!(tokens.get(cursor).map(|t|&t.tok),Some(Tok::Ident(n)) if n=="import") {
            let (import,end)=import_v2::parse_mixed(&tokens,cursor+1)?;
            let Source::Package(name)=&import.source else {
                return Err(err("E_IMPORT_PATH","generic guest requires pinned class selections"));
            };
            let provider=*available.get(name.as_str()).ok_or_else(||err("E_UNBOUND","generic package not supplied"))?;
            if import.selectors.len()!=1 {return Err(err("E_UNSUPPORTED","one generic class selection per import"));}
            let sel=&import.selectors[0];
            if sel.kind!=Kind::Class || sel.path!=provider.declaration {
                return Err(err("E_IMPORT_KIND","generic guest requires the verified class"));
            }
            if sel.binding=="main" || bindings.insert(sel.binding.clone(),provider).is_some() {
                return Err(err("E_DUPLICATE","duplicate or reserved generic binding"));
            }
            if bindings.len()>16 {return Err(err("E_SOURCE_LIMIT","generic binding budget"));}
            scope.bind_mixed(&provider.view,&import)?;
            selected.insert(provider.name.as_str(),provider);
            cursor=end;
        }
        if bindings.is_empty() {return Err(err("E_SYNTAX","generic guest requires explicit import"));}
        let owners:BTreeMap<_,_>=selected.keys().enumerate().map(|(i,name)|(*name,i)).collect();
        let views:Vec<_>=selected.values().map(|p|&p.view).collect();
        let root=&tokens[cursor..];
        // Check all original tokens before adding any internal symbols.
        if root.iter().any(|t|matches!(&t.tok,Tok::Ident(n) if n.starts_with("__atto_") || matches!(n.as_str(),"import"|"define"|"declare"|"use"|"external"|"Newable"))) {
            return Err(err("E_UNSUPPORTED","generic root cannot introduce ambient declarations or internal names"));
        }
        let mut lowered=Vec::new();
        let mut instances:BTreeMap<(&str,&str),(S,TypeIdentity)>=BTreeMap::new();
        let mut i=0;
        while i<root.len() {
            let t=&root[i];
            if let Tok::Ident(n)=&t.tok {
                if n=="new" {return Err(err("E_NEWABLE","generic allocation requires an imported scalar factory"));}
                if let Some(provider)=bindings.get(n) {
                    let (ty,scalar)=match root.get(i+2).map(|t|&t.tok) {
                        Some(Tok::Ident(s)) if s=="int"=>("int",S::Int),
                        Some(Tok::Ident(s)) if s=="bool"=>("bool",S::Bool),
                        Some(Tok::Ident(s)) if s=="string"=>("string",S::String),
                        _=>return Err(err("E_TYPE","unsupported generic scalar argument")),
                    };
                    if root.get(i+1).is_none_or(|t|t.tok!=Tok::LBracket)
                        || root.get(i+3).is_none_or(|t|t.tok!=Tok::RBracket) {
                        return Err(err("E_ARITY","generic reference requires one explicit scalar argument"));
                    }
                    let before=i.checked_sub(1).and_then(|j|root.get(j));
                    let typed=before.is_some_and(|t|t.tok==Tok::Colon);
                    let factory=!before.is_some_and(|t|t.tok==Tok::Dot)
                        && root.get(i+4).is_some_and(|t|t.tok==Tok::Dot)
                        && matches!(root.get(i+5).map(|t|&t.tok),Some(Tok::Ident(n)) if n=="new")
                        && root.get(i+6).is_some_and(|t|t.tok==Tok::LParen)
                        && root.get(i+7).is_some_and(|t|t.tok==Tok::RParen);
                    if !typed && !factory {return Err(err("E_NEWABLE","generic reference requires type position or exact factory call"));}
                    let identity=scope.resolve_type_identity(&[TypeTerm::Scalar(scalar),TypeTerm::Nominal{reference:n,arguments:1}],&views)?;
                    let key=(provider.name.as_str(),ty);
                    if let Some((_,existing))=instances.get(&key) {
                        if existing!=&identity {return Err(err("E_IMPORT_KIND","specialization identity mismatch"));}
                    } else {instances.insert(key,(scalar,identity));}
                    if instances.len()>48 {return Err(err("E_SOURCE_LIMIT","generic specialization budget"));}
                    let mut owner=t.clone();
                    owner.tok=Tok::Ident(format!("__atto_generic_class_{}_{ty}",owners[provider.name.as_str()]));
                    lowered.push(owner);
                    i+=4;
                    if factory {
                        lowered.push(root[i].clone());
                        let mut method=root[i+1].clone();
                        method.tok=Tok::Ident(SEALED_FACTORY.into());
                        lowered.push(method);
                        lowered.extend_from_slice(&root[i+2..i+4]);
                        i+=4;
                    }
                    continue;
                }
            }
            lowered.push(t.clone());
            i+=1;
        }
        if instances.is_empty() {return Err(err("E_UNSUPPORTED","no explicit generic instance selected"));}
        let parsed=crate::parser::Parser::new(lowered.clone()).parse_program()?;
        if parsed.funcs.len()!=1 || parsed.funcs[0].name!="main" || !parsed.classes.is_empty()
            || !parsed.enums.is_empty() || !parsed.actors.is_empty() || parsed.actor_interface.is_some() {
            return Err(err("E_UNSUPPORTED","generic guest requires only main"));
        }
        let mut linked=Vec::new();
        for ((package,ty),(scalar,_)) in instances {
            let provider=selected[package];
            let instance=provider.specialize(scalar)?;
            let owner=format!("__atto_generic_class_{}_{ty}",owners[package]);
            let pattern=lexer::lex(&format!("pub static fnc {SEALED_FACTORY}(): {} {{ return new {}(); }}",provider.class_name,provider.class_name))?;
            let pattern=&pattern[..pattern.len()-1];
            let matches:Vec<_>=instance.tokens.windows(pattern.len()).enumerate()
                .filter(|(_,w)|w.iter().map(|t|&t.tok).eq(pattern.iter().map(|t|&t.tok)))
                .map(|(i,_)|i).collect();
            if matches.len()!=1 {return Err(err("E_NEWABLE","missing checked specialization factory"));}
            let mut positions=BTreeSet::from([3usize]);
            for (i,t) in pattern.iter().enumerate() {
                if matches!(&t.tok,Tok::Ident(n) if n==&provider.class_name) {positions.insert(matches[0]+i);}
            }
            for (i,t) in instance.tokens.iter().enumerate().take(instance.tokens.len()-1) {
                let mut t=t.clone();
                if matches!(&t.tok,Tok::Ident(n) if n==&provider.class_name) {
                    if !positions.contains(&i) {return Err(err("E_UNSUPPORTED","arbitrary generic self-type remapping unsupported"));}
                    t.tok=Tok::Ident(owner.clone());
                }
                linked.push(t);
            }
            if linked.len()>lexer::MAX_TOKENS {return Err(err("E_SOURCE_LIMIT","generic instance token budget"));}
        }
        linked.extend(lowered);
        if linked.len()>lexer::MAX_TOKENS {return Err(err("E_SOURCE_LIMIT","generic guest token budget"));}
        let parsed=crate::parser::Parser::new(linked.clone()).parse_program()?;
        let checked=crate::check::check(&parsed)?;
        if !checked.imports.is_empty() {return Err(err("E_UNSUPPORTED","generic guest cannot request ambient host imports"));}
        crate::compile_prelexed(linked,None)
    }

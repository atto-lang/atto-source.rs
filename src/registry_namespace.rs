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
    // Class-only namespaces admitted for static identity, not executable ABI.
    class_metadata: BTreeMap<String,String>,
    // Physical owner of each admitted declaration, from pinned bytes.
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
        let mut class_metadata = BTreeMap::new();
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
            // Conservative multi-file lane: public declarations may be
            // referenced across files. Private declarations remain usable
            // only in their own pinned file; no namespace-header spoof can
            // confer private authority on a different source unit.
            for ((&file,_),tokens) in matching.iter().copied().zip(&files){
                for t in &tokens[end..] {
                    if let Tok::Ident(name)=&t.tok {
                        if owners.get(name).is_some_and(|(owner,public)|*owner!=file && !*public){
                            return Err(Diagnostic::new(t.pos,"E_PRIVATE","cross-file private declaration reference"));
                        }
                    }
                }
            }
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
            // The first source-backed nominal lane is deliberately isolated:
            // one class-only file, ordinary class AST visibility, no imports,
            // no state containers, and no external ABI or executable claim.
            let is_word = |i:usize,w:&str| matches!(normalized.get(i).map(|t|&t.tok),Some(Tok::Ident(s)) if s==w);
            let at = end + usize::from(is_word(end,"pub"));
            if matching.len()==1 && is_word(at,"define") && is_word(at+1,"class") {
                let mut body=normalized[end..].to_vec();
                if body.iter().any(|t|matches!(&t.tok,Tok::Ident(s) if s=="main")){
                    return Err(err("E_UNSUPPORTED","registry class cannot capture synthetic entry"));
                }
                let ast=crate::parser::Parser::new(body.clone()).parse_program()?;
                if ast.classes.is_empty() || !ast.funcs.is_empty() || !ast.enums.is_empty()
                    || !ast.actors.is_empty() || ast.actor_interface.is_some()
                    || ast.classes.iter().any(|c|c.state_container) {
                    return Err(err("E_UNSUPPORTED","registry nominal metadata requires a class-only source unit"));
                }
                // Check the real source with a synthetic entry, but do not
                // generate or expose an executable from this metadata lane.
                body.pop();
                body.extend(lexer::lex("pub fnc main(): int { return 0; }")?);
                let checked_ast=crate::parser::Parser::new(body).parse_program()?;
                let checked=crate::check::check(&checked_ast)?;
                if !checked.imports.is_empty(){
                    return Err(err("E_UNSUPPORTED","nominal metadata cannot request host imports"));
                }
                for class in ast.classes {
                    let path=format!("{ns}.{}",class.name);
                    view.add(&path,&path,Kind::Class,class.public)?;
                    declaration_sources.insert(path,file.to_string());
                }
                class_metadata.insert(ns.clone(),source.to_string());
                namespaces.insert(ns.clone());
                directories.insert(dir.clone(),ns.clone());
                continue;
            }
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
        Ok(Self { name: name.clone(), id, namespaces, subpaths, view, sources: verified_sources, class_metadata, declaration_sources, literals, mixed_functions })
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


/// Compile bounded frozen-registry guests: direct functions and literal constants,
/// or closed external class construction, local typed helpers, and public scalar calls.
/// Whole-package imports, typed selections and source aliases use verified
/// snapshots owning the source bytes. No filesystem/network lookup, implicit fallback, or runtime
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
            || ((word(tokens.get(start),"const") || word(tokens.get(start),"fnc") || word(tokens.get(start),"class"))
                && (start+1..end).any(|j|word(tokens.get(j),"from"))) {
            let (parsed,next)=import_v2::parse_mixed(&tokens,start)?;
            if next!=end+1 { return Err(Diagnostic::new(pos,"E_SYNTAX","invalid import extent")); }
            let Source::Package(name)=&parsed.source else {
                return Err(Diagnostic::new(pos,"E_UNSUPPORTED","registry guest requires explicit pkg domain"));
            };
            let p=*available.get(name.as_str()).ok_or_else(||Diagnostic::new(pos,"E_UNBOUND","package is not in authenticated snapshot set"))?;
            if parsed.selectors.iter().any(|s| !matches!(s.kind,Kind::Fnc|Kind::Namespace|Kind::Const|Kind::Class)) {
                return Err(Diagnostic::new(pos,"E_UNSUPPORTED","registry guest admits function, namespace, literal constant and class selectors only"));
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

    // Bounded external class lane. Each authenticated declaration gets a
    // hygienic internal class name; identical source names never coalesce.
    // Only fresh typed locals and checked public scalar calls can cross
    // the package boundary. Object escape and general ABI remain denied.
    if admitted.values().any(|p| !p.class_metadata.is_empty()) {
        if current.is_some() {return Err(err("E_UNSUPPORTED","external class guest cannot claim current-package authority"));}
        let root=&tokens[i..];
        let denied=||err("E_UNSUPPORTED","external class guest requires checked construction and public scalar calls");
        let prefix=lexer::lex("pub fnc main(): int {")?;
        let alternate=lexer::lex("pub fnc main() -> int {")?;
        let n=prefix.len()-1;
        fn scalar_args(t:&[Token], start:usize)->R<(usize,usize)> {
            let bad=||err("E_UNSUPPORTED","external class arguments must be bounded integer literals");
            if t.get(start).map(|x|&x.tok)!=Some(&Tok::LParen){return Err(bad());}
            let mut j=start+1;
            let mut count=0usize;
            if t.get(j).map(|x|&x.tok)==Some(&Tok::RParen){return Ok((j+1,0));}
            loop {
                if t.get(j).map(|x|&x.tok)==Some(&Tok::Minus){j+=1;}
                if !matches!(t.get(j).map(|x|&x.tok),Some(Tok::Int(_))){return Err(bad());}
                j+=1;count+=1;
                if count>16{return Err(err("E_SOURCE_LIMIT","external class argument budget exceeded"));}
                match t.get(j).map(|x|&x.tok) {
                    Some(Tok::RParen)=>return Ok((j+1,count)),
                    Some(Tok::Comma)=>j+=1,
                    _=>return Err(bad()),
                }
            }
        }


        type Key=(String,String);
        struct Unit {
            target:Target,
            generated:String,
            class:crate::ast::ClassDecl,
            tokens:Vec<Token>,
            public_ctor:bool,
        }
        let resolve=|reference:&str|->R<(&FrozenRegistry,Target)>{
            let resolved=scope.resolve(reference)?;
            let p=admitted.values().find(|p|resolved.target.origin==Origin::External(p.id.clone()))
                .copied().ok_or_else(||err("E_IMPORT_PATH","external class origin is not admitted"))?;
            let target=scope.resolve_export(reference,p.view())?;
            if target.kind!=Kind::Class{return Err(err("E_IMPORT_KIND","expected authenticated external class"));}
            scope.resolve_nominal(reference,p.view())?;
            Ok((p,target))
        };
        let mut units:BTreeMap<Key,Unit>=BTreeMap::new();
        let load=|p:&FrozenRegistry,target:&Target,units:&mut BTreeMap<Key,Unit>|->R<Key>{
            let key=(p.name.clone(),target.declaration.clone());
            if let Some(unit)=units.get(&key){
                if &unit.target!=target{return Err(err("E_TYPE","conflicting authenticated nominal identity"));}
                return Ok(key);
            }
            if units.len()>=16{return Err(err("E_SOURCE_LIMIT","external class declaration budget exceeded"));}
            let (ns,name)=target.declaration.rsplit_once('.').ok_or_else(denied)?;
            let source=p.class_metadata.get(ns).ok_or_else(denied)?;
            let owner=p.declaration_source(target)?;
            let provider=crate::namespace::normalize(&lexer::lex(source)?,Some(Path::new(owner)))?;
            let semi=provider.iter().position(|t|t.tok==Tok::Semi).ok_or_else(denied)?;
            let body=&provider[semi+1..];
            let mut ast=crate::parser::Parser::new(body.to_vec()).parse_program()?;
            if ast.classes.len()!=1{return Err(err("E_UNSUPPORTED","external class execution requires a single ordinary class per source unit"));}
            let class=ast.classes.pop().unwrap();
            if class.name!=name || !class.public || class.state_container{return Err(denied());}
            // No token-wide textual substitution: only the declaration
            // name is rewritten. Self-named references need typed AST
            // remapping and are explicitly unsupported in this lane.
            if body.iter().any(|t|word(Some(t),"main")){return Err(err("E_UNSUPPORTED","external class cannot capture application entry"));}
            let positions:Vec<_>=body.iter().enumerate().filter_map(|(j,t)|word(Some(t),name).then_some(j)).collect();
            if positions.len()!=1 || positions[0]<2 || !word(body.get(positions[0]-1),"class"){
                return Err(err("E_UNSUPPORTED","external self-named references require typed nominal remapping"));
            }
            let hash=crate::sha256::hex_digest(format!("{}/{}/{}",p.name,p.id.digest(),target.declaration).as_bytes());
            let generated=format!("__atto_registry_class_{}",&hash[..24]);
            if units.values().any(|u|u.generated==generated){return Err(err("E_DUPLICATE","internal nominal name collision"));}
            let public_ctor=class.ctor.as_ref().is_none_or(|ctor|
                body.windows(2).any(|w|w[0].pos==ctor.pos && word(w.first(),"pub") && word(w.get(1),"constructor")));
            let mut renamed=body[..body.len()-1].to_vec();
            let pos=renamed[positions[0]].pos;
            renamed[positions[0]]=id(&generated,pos);
            units.insert(key.clone(),Unit{target:target.clone(),generated,class,tokens:renamed,public_ctor});
            Ok(key)
        };
        let construct=|at:usize,out:&mut Vec<Token>,units:&mut BTreeMap<Key,Unit>|->R<(usize,Key)>{
            if !word(root.get(at),"new"){return Err(denied());}
            let open=root[at+1..].iter().position(|t|t.tok==Tok::LParen).map(|j|at+1+j).ok_or_else(denied)?;
            let reference=path_tokens(&root[at+1..open])?;
            let (p,target)=resolve(&reference)?;
            let key=load(p,&target,units)?;
            let unit=&units[&key];
            let (after,argc)=scalar_args(root,open)?;
            if let Some(ctor)=&unit.class.ctor {
                if !unit.public_ctor{return Err(err("E_PRIVATE","external explicit constructor must be public"));}
                if ctor.params.len()!=argc || ctor.params.iter().any(|q|q.ty!=crate::ast::Type::Int){
                    return Err(err("E_TYPE","external constructor requires matching scalar int arguments"));
                }
            }else if argc!=0{return Err(err("E_TYPE","implicit constructor takes no arguments"));}
            out.push(root[at].clone());
            out.push(id(&unit.generated,root[at+1].pos));
            out.extend_from_slice(&root[open..after]);
            Ok((after,key))
        };
        let call=|dot:usize,out:&mut Vec<Token>,class:&crate::ast::ClassDecl|->R<usize>{
            if root.get(dot).map(|x|&x.tok)!=Some(&Tok::Dot){return Err(denied());}
            let member=match root.get(dot+1).map(|x|&x.tok){
                Some(Tok::Ident(s))=>s.as_str(),_=>return Err(denied()),
            };
            if root.get(dot+2).map(|x|&x.tok)!=Some(&Tok::LParen){
                if class.fields.iter().any(|f|f.name==member && f.visibility!=crate::ast::FieldVisibility::Public){
                    return Err(err("E_PRIVATE","external class field is not public"));
                }
                return Err(denied());
            }
            let (after,argc)=scalar_args(root,dot+2)?;
            let mut candidates=class.methods.iter().filter(|m|m.name==member && !m.is_static && m.params.len()==argc);
            let method=candidates.next().ok_or_else(||err("E_UNBOUND","external instance method not found"))?;
            if candidates.next().is_some(){return Err(err("E_AMBIGUOUS","ambiguous external method"));}
            if !method.public{return Err(err("E_PRIVATE","external method is not public"));}
            if method.ret!=crate::ast::Type::Int || method.params.iter().any(|q|q.ty!=crate::ast::Type::Int){
                return Err(err("E_TYPE","external method must have scalar int signature"));
            }
            out.extend_from_slice(&root[dot..after]);
            Ok(after)
        };

        // Closed application-local helpers. Readers take exactly one
        // nominal object and return a public scalar method result. Factories
        // return a freshly constructed nominal object. No helper is exported,
        // recursive, or callable from a provider: its name is compiler-owned.
        struct Helper { generated:String, key:Key, factory:bool }
        let mut helpers:BTreeMap<String,Helper>=BTreeMap::new();
        let mut helper_tokens=Vec::new();
        let mut j=0usize;
        while word(root.get(j),"fnc") {
            if helpers.len()>=16{return Err(err("E_SOURCE_LIMIT","external class helper budget exceeded"));}
            let h=match root.get(j+1).map(|t|&t.tok){
                Some(Tok::Ident(h)) if crate::static_import_graph::valid_component(h) && h!="main" && !scope.has_root(h)=>h.clone(),
                _=>return Err(denied()),
            };
            if helpers.contains_key(&h){return Err(err("E_DUPLICATE","duplicate external class helper"));}
            let generated=format!("__atto_registry_helper_{h}");
            let pos=root[j].pos;
            if root.get(j+2).map(|t|&t.tok)!=Some(&Tok::LParen){return Err(denied());}
            helper_tokens.extend([id("fnc",pos),id(&generated,pos),token(Tok::LParen,pos)]);
            j+=3;
            let factory=root.get(j).map(|t|&t.tok)==Some(&Tok::RParen);
            let key;
            if factory {
                j+=1;
                if !matches!(root.get(j).map(|t|&t.tok),Some(Tok::Colon|Tok::Arrow)){return Err(denied());}
                j+=1;
                let brace=root[j..].iter().position(|t|t.tok==Tok::LBrace).map(|k|j+k).ok_or_else(denied)?;
                let reference=path_tokens(&root[j..brace])?;
                let (p,target)=resolve(&reference)?;
                key=load(p,&target,&mut units)?;
                helper_tokens.extend([token(Tok::RParen,pos),token(Tok::Colon,pos),id(&units[&key].generated,pos),token(Tok::LBrace,pos)]);
                j=brace+1;
                if !word(root.get(j),"return"){return Err(denied());}
                helper_tokens.push(id("return",pos));
                let (after,actual)=construct(j+1,&mut helper_tokens,&mut units)?;
                if actual!=key{return Err(err("E_TYPE","different nominal factory return identity"));}
                j=after;
            }else{
                let close=root[j..].iter().position(|t|t.tok==Tok::RParen).map(|k|j+k).ok_or_else(denied)?;
                let (param_at,type_start,type_end)=if root.get(j+1).map(|t|&t.tok)==Some(&Tok::Colon){
                    (j,j+2,close)
                }else{
                    if close<j+2{return Err(denied());}
                    (close-1,j,close-1)
                };
                if type_start>=type_end{return Err(denied());}
                let param=match root.get(param_at).map(|t|&t.tok){
                    Some(Tok::Ident(s)) if crate::static_import_graph::valid_component(s) && !scope.has_root(s)=>s.clone(),
                    _=>return Err(denied()),
                };
                let reference=path_tokens(&root[type_start..type_end])?;
                let (p,target)=resolve(&reference)?;
                key=load(p,&target,&mut units)?;
                j=close+1;
                if !matches!(root.get(j).map(|t|&t.tok),Some(Tok::Colon|Tok::Arrow))
                    || !word(root.get(j+1),"int") || root.get(j+2).map(|t|&t.tok)!=Some(&Tok::LBrace)
                    || !word(root.get(j+3),"return") || !word(root.get(j+4),&param){return Err(denied());}
                helper_tokens.extend([id(&param,pos),token(Tok::Colon,pos),id(&units[&key].generated,pos),
                    token(Tok::RParen,pos),token(Tok::Colon,pos),id("int",pos),token(Tok::LBrace,pos),id("return",pos),id(&param,pos)]);
                j=call(j+5,&mut helper_tokens,&units[&key].class)?;
            }
            if root.get(j).map(|t|&t.tok)!=Some(&Tok::Semi) || root.get(j+1).map(|t|&t.tok)!=Some(&Tok::RBrace){return Err(denied());}
            helper_tokens.extend_from_slice(&root[j..j+2]);j+=2;
            helpers.insert(h,Helper{generated,key,factory});
        }
        let main_end=j.checked_add(n).ok_or_else(denied)?;
        let main=root.get(j..main_end).ok_or_else(denied)?;
        if !(main.iter().map(|t|&t.tok).eq(prefix[..n].iter().map(|t|&t.tok))
            || main.iter().map(|t|&t.tok).eq(alternate[..n].iter().map(|t|&t.tok))){return Err(denied());}
        let mut caller=main.to_vec();
        j=main_end;
        let mut locals:BTreeMap<String,Key>=BTreeMap::new();
        while word(root.get(j),"val") || word(root.get(j),"const") {
            if locals.len()>=16{return Err(err("E_SOURCE_LIMIT","external class local budget exceeded"));}
            let local=match root.get(j+1).map(|x|&x.tok){
                Some(Tok::Ident(s)) if crate::static_import_graph::valid_component(s) && !scope.has_root(s) && !helpers.contains_key(s)=>s,
                _=>return Err(denied()),
            };
            if root.get(j+2).map(|x|&x.tok)!=Some(&Tok::Colon){return Err(denied());}
            let eq=root[j+3..].iter().position(|t|t.tok==Tok::Assign).map(|k|j+3+k).ok_or_else(denied)?;
            let annotation=path_tokens(&root[j+3..eq])?;
            let (p,target)=resolve(&annotation)?;
            let key=load(p,&target,&mut units)?;
            if locals.insert(local.clone(),key.clone()).is_some(){return Err(err("E_DUPLICATE","duplicate external class local"));}
            caller.extend_from_slice(&root[j..j+3]);
            caller.push(id(&units[&key].generated,root[j+3].pos));
            caller.push(root[eq].clone());
            if word(root.get(eq+1),"new"){
                let (after,actual)=construct(eq+1,&mut caller,&mut units)?;
                if actual!=key{return Err(err("E_TYPE","different nominal local annotation and construction identities"));}
                j=after;
            }else{
                let h=match root.get(eq+1).map(|t|&t.tok){Some(Tok::Ident(h))=>h,_=>return Err(denied())};
                let helper=helpers.get(h).filter(|h|h.factory).ok_or_else(denied)?;
                if helper.key!=key{return Err(err("E_TYPE","different nominal factory and local identities"));}
                if root.get(eq+2).map(|t|&t.tok)!=Some(&Tok::LParen)||root.get(eq+3).map(|t|&t.tok)!=Some(&Tok::RParen){return Err(denied());}
                caller.push(id(&helper.generated,root[eq+1].pos));
                caller.extend_from_slice(&root[eq+2..eq+4]);j=eq+4;
            }
            if root.get(j).map(|x|&x.tok)!=Some(&Tok::Semi){return Err(denied());}
            caller.push(root[j].clone());j+=1;
        }
        if !word(root.get(j),"return"){return Err(denied());}
        caller.push(root[j].clone());j+=1;
        if locals.is_empty(){
            let (after,key)=construct(j,&mut caller,&mut units)?;
            j=call(after,&mut caller,&units[&key].class)?;
        }else{
            let mut count=0usize;
            let mut moved=BTreeSet::new();
            loop {
                let name=match root.get(j).map(|x|&x.tok){Some(Tok::Ident(s))=>s,_=>return Err(denied())};
                if let Some(helper)=helpers.get(name).filter(|h|!h.factory){
                    if root.get(j+1).map(|t|&t.tok)!=Some(&Tok::LParen)||root.get(j+3).map(|t|&t.tok)!=Some(&Tok::RParen){return Err(denied());}
                    let local=match root.get(j+2).map(|t|&t.tok){Some(Tok::Ident(s))=>s,_=>return Err(denied())};
                    let actual=locals.remove(local).ok_or_else(||err("E_OWNERSHIP","external object argument is unknown or already moved"))?;
                    if actual!=helper.key{return Err(err("E_TYPE","different nominal helper argument identity"));}
                    moved.insert(local.clone());
                    caller.push(id(&helper.generated,root[j].pos));
                    caller.extend_from_slice(&root[j+1..j+4]);j+=4;
                }else{
                    if moved.contains(name){return Err(err("E_OWNERSHIP","external object already moved to helper"));}
                    let key=locals.get(name).ok_or_else(denied)?;
                    caller.push(id(name,root[j].pos));
                    j=call(j+1,&mut caller,&units[key].class)?;
                }
                count+=1;
                if count>32{return Err(err("E_SOURCE_LIMIT","external class call budget exceeded"));}
                if root.get(j).map(|x|&x.tok)!=Some(&Tok::Plus){break;}
                caller.push(root[j].clone());j+=1;
            }
        }
        if !root[j..].iter().map(|t|&t.tok).eq([Tok::Semi,Tok::RBrace,Tok::Eof].iter()){
            return Err(denied());
        }
        // Preserve the previous fail-closed profile for unused admitted
        // packages; their unlinked code must not gain ambient authority.
        if admitted.keys().any(|p|!units.keys().any(|(name,_)|name.as_str()==*p)){
            return Err(err("E_UNSUPPORTED","external class guest has unused admitted package"));
        }
        caller.extend_from_slice(&root[j..]);
        let mut linked=Vec::new();
        for unit in units.into_values(){linked.extend(unit.tokens);}
        linked.extend(helper_tokens);
        linked.extend(caller);
        if linked.len()>lexer::MAX_TOKENS{return Err(err("E_SOURCE_LIMIT","external class guest token budget exceeded"));}
        let parsed=crate::parser::Parser::new(linked.clone()).parse_program()?;
        let checked=crate::check::check(&parsed)?;
        if !checked.imports.is_empty(){return Err(err("E_UNSUPPORTED","external class guest cannot request host imports"));}
        return crate::compile_prelexed(linked,None);
    }

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

//! Frozen registry namespace resolver and bounded direct-function guest linker.
//! Explicitly bounded profile: one source file per exported directory,
//! direct functions only, declared namespace equals manifest namespace key.
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
            let mut matching = sources.iter().filter(|(path,_)| path.starts_with(dir.as_str()));
            let (&file,&source) = matching.next().ok_or_else(|| err("E_MANIFEST", "namespace directory has no pinned source"))?;
            if matching.next().is_some() || file[dir.len()..].contains('/') {
                return Err(err("E_UNSUPPORTED", "namespace profile requires exactly one direct pinned file per directory"));
            }
            let (declared, functions) = crate::import_link::registry_function_exports(source, Path::new(file))?;
            if declared != *ns { return Err(err("E_MANIFEST", "source namespace differs from manifest key")); }
            view.add(ns, ns, Kind::Namespace, true)?;
            for (function, public) in functions {
                let path = format!("{ns}.{function}");
                view.add(&path, &path, Kind::Fnc, public)?;
            }
            verified_sources.insert(ns.clone(), (file.to_string(), source.to_string()));
            namespaces.insert(ns.clone());
            directories.insert(dir.clone(),ns.clone());
        }
        let mut subpaths = BTreeMap::new();
        for (sub, dir) in parsed.exports {
            let target = directories.get(&dir)
                .ok_or_else(|| err("E_MANIFEST", "subpath targets undeclared namespace directory"))?;
            subpaths.insert(sub,target.clone());
        }
        Ok(Self { name: name.clone(), id, namespaces, subpaths, view, sources: verified_sources })
    }

    pub fn view(&self) -> &ExportView { &self.view }

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
    if source.len()>crate::SOURCE_CAP || packages.is_empty() || packages.len()>16 {
        return Err(err("E_SOURCE_LIMIT","registry guest source or package budget exceeded"));
    }
    let mut available=BTreeMap::new();
    for &p in packages {
        if available.insert(p.name.as_str(),p).is_some() {
            return Err(err("E_AMBIGUOUS","multiple pinned snapshots for one registry name"));
        }
    }
    let tokens=lexer::lex(source)?;
    for t in &tokens {
        if matches!(&t.tok,Tok::Ident(s) if s.starts_with("__atto_")) {
            return Err(Diagnostic::new(t.pos,"E_RESERVED","reserved compiler-generated identifier"));
        }
    }
    let mut scope=Scope::new();
    let mut admitted=BTreeMap::new();
    let mut i=0;
    // This intentionally narrow guest API does not infer a package-relative
    // file namespace; callers must use the single-file root profile.
    while word(tokens.get(i),"import") {
        let pos=tokens[i].pos;
        let start=i+1;
        let mut end=start;
        while tokens.get(end).is_some_and(|t| !matches!(t.tok,Tok::Semi|Tok::Eof)) { end+=1; }
        if tokens.get(end).is_none_or(|t|t.tok!=Tok::Semi) {
            return Err(Diagnostic::new(pos,"E_SYNTAX","unterminated registry import"));
        }
        if word(tokens.get(start),"pkg") {
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
        } else if import_v2::is_mixed(&tokens,start) {
            let (parsed,next)=import_v2::parse_mixed(&tokens,start)?;
            if next!=end+1 { return Err(Diagnostic::new(pos,"E_SYNTAX","invalid import extent")); }
            let Source::Package(name)=&parsed.source else {
                return Err(Diagnostic::new(pos,"E_UNSUPPORTED","registry guest requires explicit pkg domain"));
            };
            let p=*available.get(name.as_str()).ok_or_else(||Diagnostic::new(pos,"E_UNBOUND","package is not in authenticated snapshot set"))?;
            if parsed.selectors.iter().any(|s| !matches!(s.kind,Kind::Fnc|Kind::Namespace)) {
                return Err(Diagnostic::new(pos,"E_UNSUPPORTED","registry guest admits function and namespace selectors only"));
            }
            scope.bind_mixed(p.view(),&parsed).map_err(|d|at(d,pos))?;
            admitted.insert(p.name.as_str(),p);
        } else if word(tokens.get(start),"namespace") {
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
            return Err(Diagnostic::new(pos,"E_UNSUPPORTED","unsupported registry guest import clause"));
        }
        i=end+1;
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
            let funcs=&normalized[semi+1..];
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
                            && !matches!(s.as_str(),"if"|"while"|"match"|"for")) {
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
                if tokens.get(end).is_none_or(|x|x.tok!=Tok::LParen) {
                    return Err(Diagnostic::new(t.pos,"E_UNSUPPORTED","registry binding supports direct calls only"));
                }
                let reference=path_tokens(&tokens[start..end]).map_err(|d|at(d,t.pos))?;
                let resolved=scope.resolve(&reference).map_err(|d|at(d,t.pos))?;
                let p=admitted.values().find(|p|resolved.target.origin==Origin::External(p.id.clone()))
                    .ok_or_else(||Diagnostic::new(t.pos,"E_IMPORT_PATH","resolved origin is not admitted"))?;
                let target=scope.resolve_export(&reference,p.view()).map_err(|d|at(d,t.pos))?;
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

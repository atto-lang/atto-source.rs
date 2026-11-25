//! Frozen registry namespace resolver, not acquisition or guest codegen.
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
            namespaces.insert(ns.clone());
            directories.insert(dir.clone(),ns.clone());
        }
        let mut subpaths = BTreeMap::new();
        for (sub, dir) in parsed.exports {
            let target = directories.get(&dir)
                .ok_or_else(|| err("E_MANIFEST", "subpath targets undeclared namespace directory"))?;
            subpaths.insert(sub,target.clone());
        }
        Ok(Self { name: name.clone(), id, namespaces, subpaths, view })
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
        if !namespace(selector) || !namespace(binding) {
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

//! Static import v2 binding graph (host-side resolver, not guest codegen).
//! Package identity and export records MUST come from an authenticated locked
//! source graph. This module performs no acquisition, I/O or capability grant.
//! No public parser input may be treated as proof of builtin provenance.
use crate::{Diagnostic, Pos};
use crate::import_v2::{Kind, MixedImport, Source};
use std::collections::{BTreeMap, BTreeSet};
type R<T> = Result<T, Diagnostic>;
const LIMIT: usize = 256;
const MAX_PATH: usize = 8;

fn err(code: &'static str, msg: &str) -> Diagnostic {
    Diagnostic::new(Pos::default(), code, msg)
}

pub(crate) fn valid_component(s: &str) -> bool { ident(s) }

fn ident(s: &str) -> bool {
    !s.is_empty() && s.len() <= 64 && s.is_ascii()
        && s.as_bytes().first().is_some_and(|c| c.is_ascii_alphabetic() || *c == b'_')
        && s.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_')
        && !s.starts_with("__atto_") && !crate::parser::is_reserved(s)
        && !matches!(s, "pkg" | "enum" | "use" | "declare" | "mtd")
}
fn path(s: &str) -> bool {
    s.split('.').count() <= MAX_PATH && s.split('.').all(ident)
}
fn prefix(a: &str, b: &str) -> bool {
    b.strip_prefix(a).is_some_and(|rest| rest.starts_with('.'))
}
fn namespace(kind: Kind) -> bool { matches!(kind, Kind::Namespace | Kind::Module) }

/// Check a normalized complete alias dependency inventory without consulting
/// any export table, so even source-less and not-yet-linked package aliases
/// cannot hide cycles behind a loader or an unresolved reference.
fn reject_alias_cycles(pending: &BTreeMap<&str, &str>) -> R<()> {
    // Every alias depends on at most one other alias: the first component
    // of its qualified RHS. Detect cycles in the entire declaration graph
    // before attempting any export lookup. Iterative walks avoid recursive
    // stack growth and give a deterministic cycle path even for unused
    // declarations or qualified references such as a = b.child.
    let mut checked = BTreeSet::new();
    for &start in pending.keys() {
        let mut walk = Vec::new();
        let mut positions = BTreeMap::new();
        let mut current = start;
        while let Some(&reference) = pending.get(current) {
            if checked.contains(current) { break; }
            if let Some(&at) = positions.get(current) {
                let mut cycle: Vec<&str> = walk[at..].to_vec();
                cycle.push(current);
                return Err(err("E_IMPORT_CYCLE", &format!("cyclic namespace aliases: {}", cycle.join(" -> "))));
            }
            positions.insert(current, walk.len());
            walk.push(current);
            current = reference.split('.').next().unwrap_or("");
        }
        checked.extend(walk);
    }
    Ok(())
}

pub(crate) fn precheck_alias_declarations(declarations: &[(&str, &str)]) -> R<()> {
    if declarations.len() > LIMIT { return Err(err("E_SOURCE_LIMIT", "too many namespace aliases")); }
    let mut pending = BTreeMap::new();
    for &(name, reference) in declarations {
        if !ident(name) || !path(reference) {
            return Err(err("E_IMPORT_PATH", "invalid namespace alias declaration"));
        }
        if pending.insert(name, reference).is_some() {
            return Err(err("E_DUPLICATE", "duplicate namespace alias"));
        }
    }
    reject_alias_cycles(&pending)
}

/// Canonical registry identity plus authenticated content digest.
/// Registry spelling is retained, never normalized into a trust decision.
/// Construction validates structure only; authentication belongs upstream.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct PackageId { name: String, version: String, digest: String }
impl PackageId {
    pub fn new(name: &str, version: &str, digest: &str) -> R<Self> {
        let parts: Vec<_> = name.strip_prefix('@').unwrap_or("").split('/').collect();
        if parts.len() != 3 || !parts.iter().all(|p| !p.is_empty() && p.len() <= 64
            && p.as_bytes()[0].is_ascii_alphanumeric()
            && p.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_'))
            || version.is_empty() || version.len() > 64
            || !version.bytes().all(|c| c.is_ascii_alphanumeric() || matches!(c, b'.' | b'-' | b'+'))
            || digest.len() != 64 || !digest.bytes().all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
        { return Err(err("E_IMPORT_PATH", "invalid pinned canonical package identity")); }
        Ok(Self { name: name.into(), version: version.into(), digest: digest.into() })
    }
    pub fn suffixes(&self) -> Vec<String> {
        let parts: Vec<_> = self.name[1..].split('/').map(|p| p.replace('-', "_")).collect();
        (0..parts.len()).rev().map(|i| parts[i..].join(".")).filter(|p| path(p)).collect()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Origin { Builtin(String), External(PackageId), Local(String) }
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Target {
    pub origin: Origin,
    /// Stable declaration path supplied by the checked export graph.
    pub declaration: String,
    pub kind: Kind,
}
/// Opaque resolver identity; not a runtime type descriptor.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NominalId(Target);

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Tier { Implicit, Deliberate, Explicit }

#[derive(Clone, Debug, PartialEq, Eq)]
struct Binding { name: String, tier: Tier, target: Target }

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Resolved { pub target: Target, pub remainder: String }

#[derive(Clone, Debug, Default)]
pub struct Scope { bindings: Vec<Binding> }

impl Scope {
    pub fn new() -> Self { Self::default() }

    pub(crate) fn has_root(&self, name: &str) -> bool {
        self.bindings.iter().any(|b| b.name.split('.').next() == Some(name))
    }

    fn insert(&mut self, name: &str, tier: Tier, target: Target) -> R<()> {
        if !path(name) { return Err(err("E_IMPORT_PATH", "invalid static binding path")); }
        if self.bindings.len() >= LIMIT { return Err(err("E_SOURCE_LIMIT", "static binding budget exceeded")); }
        for b in &self.bindings {
            if b.tier == Tier::Explicit && tier == Tier::Explicit && b.name == name {
                return Err(err("E_DUPLICATE", "duplicate explicit binding"));
            }
            if (b.tier == tier || b.tier == Tier::Explicit || tier == Tier::Explicit)
                && ((!namespace(b.target.kind) && prefix(&b.name, name))
                || (!namespace(target.kind) && prefix(name, &b.name))) {
                return Err(err("E_DUPLICATE", "non-namespace obstructs static binding prefix"));
            }
        }
        self.bindings.push(Binding { name: name.into(), tier, target });
        Ok(())
    }

    /// Admit a caller-verified package root. All suffixes are candidates;
    /// same-tier distinct targets collide only when referenced.
    pub fn package(&mut self, id: PackageId, alias: Option<&str>) -> R<()> {
        let names = alias.map_or_else(|| id.suffixes(), |s| vec![s.to_string()]);
        if names.is_empty() { return Err(err("E_IMPORT_PATH", "package needs explicit alias")); }
        let target = Target { origin: Origin::External(id), declaration: String::new(), kind: Kind::Namespace };
        let tier = if alias.is_some() { Tier::Explicit } else { Tier::Deliberate };
        let mut staged = self.clone();
        for name in names { staged.insert(&name, tier, target.clone())?; }
        *self = staged;
        Ok(())
    }

    /// A verified builtin or lexical declaration is supplied by the caller.
    /// This function cannot authenticate origin or grant intrinsic privileges.
    pub fn bind_verified(&mut self, name: &str, tier: Tier, target: Target) -> R<()> {
        self.insert(name, tier, target)
    }

    /// Choose highest populated tier before looking up members.
    /// Missing members never select a lower tier or another package.
    pub fn resolve(&self, reference: &str) -> R<Resolved> {
        if !path(reference) { return Err(err("E_IMPORT_PATH", "invalid static reference")); }
        let mut candidates: Vec<_> = self.bindings.iter().filter(|b| b.name == reference || prefix(&b.name, reference)).collect();
        let tier = candidates.iter().map(|b| b.tier).max()
            .ok_or_else(|| err("E_UNBOUND", "unbound static import reference"))?;
        candidates.retain(|b| b.tier == tier);
        let first = candidates[0];
        let remainder = reference[first.name.len()..].trim_start_matches('.');
        if candidates.iter().any(|b| b.target != first.target
            || reference[b.name.len()..].trim_start_matches('.') != remainder) {
            return Err(err("E_AMBIGUOUS", "ambiguous static import prefix"));
        }
        if !remainder.is_empty() && !namespace(first.target.kind) {
            return Err(err("E_IMPORT_KIND", "non-namespace is not a static prefix"));
        }
        Ok(Resolved { target: first.target.clone(), remainder: remainder.into() })
    }

    /// Resolve a member against the same verified origin, not another view.
    /// This bounded lane requires a canonical public declaration path in the
    /// export view; remapped member trees need the full authenticated graph.
    pub fn resolve_export(&self, reference: &str, view: &ExportView) -> R<Target> {
        let resolved = self.resolve(reference)?;
        if resolved.target.origin != view.origin {
            return Err(err("E_IMPORT_PATH", "export view does not match selected origin"));
        }
        // Revalidate exact bindings too: a lexical alias or a caller-supplied
        // Target is not proof that a declaration remains public in this view.
        let base = &resolved.target.declaration;
        if base.is_empty() {
            if resolved.target.kind != Kind::Namespace || !matches!(&resolved.target.origin, Origin::External(_)) {
                return Err(err("E_IMPORT_KIND", "only external package namespaces have an empty declaration"));
            }
        } else if view.lookup(base)? != resolved.target {
            return Err(err("E_IMPORT_KIND", "canonical declaration view mismatch"));
        }
        if resolved.remainder.is_empty() { return Ok(resolved.target); }
        let key = if base.is_empty() { resolved.remainder }
            else { format!("{base}.{}", resolved.remainder) };
        view.lookup(&key)
    }

    /// Resolve only in the uniquely selected origin's authenticated view.
    pub fn resolve_in_views(&self, reference: &str, views: &[&ExportView]) -> R<Target> {
        let resolved = self.resolve(reference)?;
        let mut matching = views.iter().filter(|v| v.origin == resolved.target.origin);
        let view = matching.next().ok_or_else(|| err("E_UNBOUND", "missing authenticated export view"))?;
        if matching.next().is_some() { return Err(err("E_AMBIGUOUS", "duplicate authenticated export view")); }
        self.resolve_export(reference, view)
    }

    /// Resolve a nominal declaration identity, not its runtime layout or ABI.
    /// General type aliases are intentionally excluded until their expansion
    /// rules are checked. The authenticated package digest is part of identity.
    pub fn resolve_nominal(&self, reference: &str, view: &ExportView) -> R<NominalId> {
        let target = self.resolve_export(reference, view)?;
        if !matches!(target.kind, Kind::Class | Kind::Enum | Kind::Actor) {
            return Err(err("E_IMPORT_KIND", "expected nominal class, enum or actor"));
        }
        Ok(NominalId(target))
    }

    /// Create a namespace alias after resolution of a checked namespace node.
    /// Chains of already resolved constants work; forward/cyclic references
    /// fail closed pending a separate graph-wide alias topological pass.
    pub fn namespace_alias(&mut self, alias: &str, target: Target) -> R<()> {
        if !namespace(target.kind) { return Err(err("E_IMPORT_KIND", "namespace alias requires namespace or module")); }
        self.insert(alias, Tier::Explicit, target)
    }

    /// Batch namespace aliases, with forward references and atomic
    /// publication. Pending explicit names shadow lower tiers from the
    /// start; never use a lower-tier binding to break an alias cycle.
    /// Interior namespace resolution is restricted to verified views.
    pub fn namespace_aliases(&mut self, declarations: &[(&str, &str)], views: &[&ExportView]) -> R<()> {
        if declarations.len() > LIMIT { return Err(err("E_SOURCE_LIMIT", "too many namespace aliases")); }
        let mut staged = self.clone();
        let mut pending = BTreeMap::new();
        for &(name, reference) in declarations {
            if !ident(name) || !path(reference) {
                return Err(err("E_IMPORT_PATH", "invalid namespace alias declaration"));
            }
            if pending.insert(name, reference).is_some() || staged.bindings.iter()
                .any(|b| b.name == name && b.tier == Tier::Explicit) {
                return Err(err("E_DUPLICATE", "duplicate namespace alias"));
            }
        }
        if staged.bindings.len().saturating_add(pending.len()) > LIMIT {
            return Err(err("E_SOURCE_LIMIT", "static binding budget exceeded"));
        }
        reject_alias_cycles(&pending)?;
        while !pending.is_empty() {
            let mut ready = Vec::new();
            for (&name, &reference) in &pending {
                if pending.keys().any(|p| *p == reference || prefix(p, reference)) { continue; }
                let r = staged.resolve(reference)?;
                let target = if r.remainder.is_empty() { r.target } else {
                    let mut matches = views.iter().filter(|v| v.origin == r.target.origin);
                    let view = matches.next().ok_or_else(|| err("E_UNBOUND", "missing verified namespace view"))?;
                    if matches.next().is_some() { return Err(err("E_AMBIGUOUS", "duplicate verified namespace view")); }
                    staged.resolve_export(reference, view)?
                };
                if !namespace(target.kind) {
                    return Err(err("E_IMPORT_KIND", "namespace alias requires namespace"));
                }
                ready.push((name, target));
            }
            if ready.is_empty() { return Err(err("E_IMPORT_CYCLE", "cyclic namespace aliases")); }
            for (name, target) in ready {
                staged.namespace_alias(name, target)?;
                pending.remove(name);
            }
        }
        *self = staged;
        Ok(())
    }

    /// Resolve the entire typed selector list against a verified export view
    /// before publishing any bindings. No source fallback or kind inference.
    pub fn bind_mixed(&mut self, exports: &ExportView, import: &MixedImport) -> R<()> {
        // This slice only binds exact already-verified root views. Manifest
        // subpath and URL resolution must supply a verified view mapping
        // before they can be admitted; never guess from a lexical selector.
        let same_source = match (&import.source, &exports.origin) {
            (Source::Package(name), Origin::External(id)) => name == &id.name,
            (Source::Builtin(name), Origin::Builtin(id)) => name == id,
            (Source::Local(name), Origin::Local(id)) => name == id,
            _ => false,
        };
        if !same_source { return Err(err("E_IMPORT_PATH", "export view does not match requested source domain and identity")); }
        let selectors = &import.selectors;
        if selectors.is_empty() || selectors.len() > LIMIT {
            return Err(err("E_SOURCE_LIMIT", "invalid typed selection count"));
        }
        let mut staged = self.clone();
        let mut seen = BTreeSet::new();
        for selector in selectors {
            if !seen.insert(&selector.binding) { return Err(err("E_DUPLICATE", "duplicate mixed binding")); }
            let target = exports.lookup(&selector.path)?;
            if target.kind != selector.kind { return Err(err("E_IMPORT_KIND", "export kind does not match typed selector")); }
            let tier = if selector.explicit_alias { Tier::Explicit } else { Tier::Deliberate };
            staged.insert(&selector.binding, tier, target)?;
        }
        *self = staged;
        Ok(())
    }
}

#[derive(Clone, Debug)]
struct Export { target: Target, public: bool }
/// Caller-verified source-owned export graph view. No filesystem walking.
/// Parent namespace visibility must be represented in the supplied view.
pub struct ExportView { origin: Origin, entries: BTreeMap<String, Export> }
impl ExportView {
    pub fn new(origin: Origin) -> Self { Self { origin, entries: BTreeMap::new() } }

    pub fn add(&mut self, export: &str, declaration: &str, kind: Kind, public: bool) -> R<()> {
        if !path(export) || !path(declaration) { return Err(err("E_IMPORT_PATH", "invalid export identity")); }
        if self.entries.len() >= LIMIT { return Err(err("E_SOURCE_LIMIT", "export view budget exceeded")); }
        if self.entries.contains_key(export) { return Err(err("E_DUPLICATE", "duplicate export")); }
        self.entries.insert(export.into(), Export { target: Target {
            origin: self.origin.clone(), declaration: declaration.into(), kind }, public });
        Ok(())
    }

    pub fn lookup(&self, export: &str) -> R<Target> {
        if !path(export) { return Err(err("E_IMPORT_PATH", "invalid export path")); }
        let found = self.entries.get(export).ok_or_else(|| err("E_UNBOUND", "unknown export"))?;
        if !found.public { return Err(err("E_PRIVATE", "private export")); }
        for (i, _) in export.match_indices('.') {
            let parent = self.entries.get(&export[..i]).ok_or_else(|| err("E_UNBOUND", "missing parent namespace"))?;
            if !parent.public { return Err(err("E_PRIVATE", "private parent namespace")); }
            if !namespace(parent.target.kind) { return Err(err("E_IMPORT_KIND", "non-namespace export prefix")); }
        }
        Ok(found.target.clone())
    }
}

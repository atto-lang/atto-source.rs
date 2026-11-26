//! Incremental compilation v1: content-addressed build keys, a bounded
//! in-memory artifact cache, and reverse-dependency invalidation.
//!
//! Soundness rule: a cache hit is allowed only when every input that can
//! influence the guest artifact is byte-identical. The key therefore covers
//! the compiler version, ABI version, entry path, and for every *supplied*
//! unit its path, package identity and SHA-256 source digest. Every supplied
//! unit counts, not only reachable ones, because `compile_linked` rejects an
//! unreachable unit (E_UNBOUND), so its result depends on the whole input
//! set. Key order is canonical (sorted by path), so the caller's unit order
//! never matters. There are no timestamps, no mtime heuristics and no
//! interface-only early cutoff yet (see docs/incremental-compilation-v1.md).
//!
//! Diagnostics are cached too, so an editor re-checking an unchanged broken
//! graph gets the identical diagnostic without re-running the compiler.
use crate::diag::{Diagnostic, Pos};
use crate::linker::{self, LinkUnit};
use crate::sha256;
use std::collections::{BTreeMap, BTreeSet, VecDeque};

type R<T> = Result<T, Diagnostic>;

const KEY_DOMAIN: &[u8] = b"atto-incremental-build-key-v2\0";
const MAX_UNITS: usize = 64;
pub const DEFAULT_CAPACITY: usize = 64;
pub const MAX_CAPACITY: usize = 4096;

fn err(code: &'static str, message: impl Into<String>) -> Diagnostic {
    Diagnostic::new(Pos::default(), code, message)
}

/// 32-byte content-addressed identity of one build request.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct BuildKey(pub [u8; 32]);

impl BuildKey {
    pub fn hex(&self) -> String {
        self.0.iter().map(|b| format!("{b:02x}")).collect()
    }
}

/// Import graph of the supplied units, keyed by canonical path.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ImportGraph {
    pub edges: BTreeMap<String, Vec<String>>,
}

impl ImportGraph {
    pub fn build(units: &[LinkUnit<'_>]) -> R<Self> {
        if units.is_empty() || units.len() > MAX_UNITS {
            return Err(err("E_SOURCE_LIMIT", "invalid incremental source-unit count"));
        }
        let mut edges = BTreeMap::new();
        for unit in units {
            if unit.source.len() > crate::SOURCE_CAP {
                return Err(err("E_SOURCE_LIMIT", "source unit exceeds browser source limit"));
            }
            let imports = linker::import_paths(unit.source, unit.path)?;
            if edges.insert(unit.path.to_string(), imports).is_some() {
                return Err(err("E_DUPLICATE", "duplicate canonical source file path"));
            }
        }
        Ok(ImportGraph { edges })
    }

    /// Units reachable from `entry` (inclusive), in sorted order.
    pub fn reachable(&self, entry: &str) -> R<BTreeSet<String>> {
        if !self.edges.contains_key(entry) {
            return Err(err("E_UNBOUND", "entry file does not exist in the supplied source graph"));
        }
        let mut seen = BTreeSet::new();
        let mut pending = vec![entry.to_string()];
        while let Some(path) = pending.pop() {
            if !seen.insert(path.clone()) {
                continue;
            }
            if let Some(next) = self.edges.get(&path) {
                pending.extend(next.iter().cloned());
            }
        }
        Ok(seen)
    }

    /// `changed` plus every unit that transitively imports a changed unit.
    /// This is the set whose diagnostics/artifacts an editor must refresh.
    pub fn affected_by(&self, changed: &BTreeSet<String>) -> BTreeSet<String> {
        let mut reverse: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
        for (from, tos) in &self.edges {
            for to in tos {
                reverse.entry(to.as_str()).or_default().push(from.as_str());
            }
        }
        let mut out = BTreeSet::new();
        let mut pending: Vec<String> = changed.iter().cloned().collect();
        while let Some(path) = pending.pop() {
            if !out.insert(path.clone()) {
                continue;
            }
            if let Some(importers) = reverse.get(path.as_str()) {
                pending.extend(importers.iter().map(|s| s.to_string()));
            }
        }
        out
    }
}

fn push_field(buf: &mut Vec<u8>, bytes: &[u8]) {
    buf.extend_from_slice(&(bytes.len() as u64).to_le_bytes());
    buf.extend_from_slice(bytes);
}

/// Compute the build key for `entry` over `units`. Length-prefixed fields
/// make the encoding injective (no path/source concatenation ambiguity).
pub fn build_key(entry: &str, units: &[LinkUnit<'_>]) -> R<BuildKey> {
    let graph = ImportGraph::build(units)?;
    build_key_with_graph(entry, units, &graph)
}

fn build_key_with_graph(entry: &str, units: &[LinkUnit<'_>], graph: &ImportGraph) -> R<BuildKey> {
    // Validates the entry exists; the graph also proves paths are unique.
    graph.reachable(entry)?;
    let by_path: BTreeMap<&str, &LinkUnit<'_>> = units.iter().map(|u| (u.path, u)).collect();
    let mut buf = Vec::with_capacity(256);
    buf.extend_from_slice(KEY_DOMAIN);
    push_field(&mut buf, crate::COMPILER_VERSION.as_bytes());
    push_field(&mut buf, &crate::ABI_VERSION.to_le_bytes());
    push_field(&mut buf, match crate::library::target_os() {
        crate::library::TargetOs::Posix => b"posix",
        crate::library::TargetOs::Windows => b"windows",
    });
    push_field(&mut buf, entry.as_bytes());
    push_field(&mut buf, &(by_path.len() as u64).to_le_bytes());
    for (path, unit) in &by_path {
        push_field(&mut buf, path.as_bytes());
        push_field(&mut buf, unit.package.as_bytes());
        push_field(&mut buf, &sha256::sha256(unit.source.as_bytes()));
    }
    Ok(BuildKey(sha256::sha256(&buf)))
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CacheStats {
    pub hits: u64,
    pub misses: u64,
    pub evictions: u64,
}

/// Bounded FIFO cache of compiled guest artifacts and diagnostics.
pub struct IncrementalCompiler {
    capacity: usize,
    entries: BTreeMap<BuildKey, (Vec<String>, R<Vec<u8>>)>,
    order: VecDeque<BuildKey>,
    last_digests: BTreeMap<String, [u8; 32]>,
    stats: CacheStats,
}

impl Default for IncrementalCompiler {
    fn default() -> Self {
        Self::with_capacity(DEFAULT_CAPACITY).expect("default capacity is valid")
    }
}

impl IncrementalCompiler {
    pub fn with_capacity(capacity: usize) -> R<Self> {
        if capacity == 0 || capacity > MAX_CAPACITY {
            return Err(err("E_CACHE", "incremental cache capacity must be in [1, 4096]"));
        }
        Ok(IncrementalCompiler {
            capacity,
            entries: BTreeMap::new(),
            order: VecDeque::new(),
            last_digests: BTreeMap::new(),
            stats: CacheStats::default(),
        })
    }

    pub fn stats(&self) -> CacheStats {
        self.stats
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Compile through the cache. A hit returns a byte-identical artifact
    /// (or the identical diagnostic) without invoking the compiler.
    pub fn compile(&mut self, entry: &str, units: &[LinkUnit<'_>]) -> R<Vec<u8>> {
        // Snapshot platform configuration once, including concurrent changes
        // to the host's default. Never publish an artifact under a different
        // OS key than the compiler actually used.
        let target = crate::library::target_os();
        crate::library::with_target_os(target, || self.compile_scoped(entry, units))
    }

    fn compile_scoped(&mut self, entry: &str, units: &[LinkUnit<'_>]) -> R<Vec<u8>> {
        // A cached artifact cannot recreate ownership evidence side effects.
        if crate::borrow_export::enabled() {
            return linker::compile_linked(entry, units);
        }
        let key = build_key(entry, units)?;
        let paths: Vec<String> = units.iter().map(|u| u.path.to_string()).collect();
        if let Some((prior_order, found)) = self.entries.get(&key) {
            // The linker may report the first invalid supplied input in
            // caller order. Never serve a diagnostic from another ordering.
            if found.is_ok() || prior_order == &paths {
                self.stats.hits += 1;
                return found.clone();
            }
        }
        if self.entries.remove(&key).is_some() {
            self.order.retain(|old| old != &key);
        }
        self.stats.misses += 1;
        let result = linker::compile_linked(entry, units);
        if self.entries.len() >= self.capacity {
            if let Some(oldest) = self.order.pop_front() {
                self.entries.remove(&oldest);
                self.stats.evictions += 1;
            }
        }
        self.entries.insert(key, (paths, result.clone()));
        self.order.push_back(key);
        result
    }

    /// Paths whose content differs from the previous call (added files
    /// count as changed; removed files are reported too), then records the
    /// new digests. Pair with `ImportGraph::affected_by` for editor refresh.
    pub fn changed_since_last(&mut self, units: &[LinkUnit<'_>]) -> BTreeSet<String> {
        let now: BTreeMap<String, [u8; 32]> = units
            .iter()
            .map(|u| {
                let mut identity = Vec::new();
                push_field(&mut identity, u.package.as_bytes());
                push_field(&mut identity, u.source.as_bytes());
                (u.path.to_string(), sha256::sha256(&identity))
            })
            .collect();
        let mut changed = BTreeSet::new();
        for (path, digest) in &now {
            if self.last_digests.get(path) != Some(digest) {
                changed.insert(path.clone());
            }
        }
        for path in self.last_digests.keys() {
            if !now.contains_key(path) {
                changed.insert(path.clone());
            }
        }
        self.last_digests = now;
        changed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::library::{with_target_os, TargetOs};
    #[test]
    fn platform_is_part_of_key_and_scope_restores() {
        let units = [LinkUnit { path: "main.atto", package: "a@1.0.0", source: "pub fnc main(): int { return 42; }" }];
        let a = with_target_os(TargetOs::Posix, || build_key("main.atto", &units).unwrap());
        let b = with_target_os(TargetOs::Windows, || {
            assert_eq!(crate::library::target_os(), TargetOs::Windows);
            assert_eq!(with_target_os(TargetOs::Posix, crate::library::target_os), TargetOs::Posix);
            assert_eq!(crate::library::target_os(), TargetOs::Windows);
            build_key("main.atto", &units).unwrap()
        });
        assert_ne!(a,b);
    }
}

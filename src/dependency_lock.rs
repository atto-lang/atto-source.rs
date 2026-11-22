//! Dependency lockfile v1 (`atto.lock`): locked versions, content-verified
//! vendored artifacts, offline builds and a deterministic install order.
//!
//! This extends the existing single-package `.atto.pkg.lock` source pin
//! (package_resolver.rs) to cross-package dependencies. Design rules:
//!
//! * **Canonical form only.** A lockfile is accepted only if it is
//!   byte-identical to `render(parse(text))`. Two toolchains that agree on
//!   the dependency set therefore always produce the same lock bytes, and
//!   reviewers see no formatting-only diffs.
//! * **Content addressing.** Each package pins a `tree_sha256` over its
//!   canonical file tree (sorted, length-prefixed, domain-separated).
//! * **Offline is a mode, not a fallback.** With `offline = true`, every
//!   package must be satisfied by a vendored tree whose digest matches;
//!   nothing is fetched and a registry-only package fails closed.
//! * **Bounded and acyclic.** At most 512 packages, a single version per
//!   name (no silent duplicate resolution), and no dependency cycles.
//!
//! Authenticity still comes from a trusted lock digest supplied
//! out-of-band (as with compile_package_locked); an unsigned lock next to
//! mutable sources is integrity, not authentication.
use crate::diag::{Diagnostic, Pos};
use crate::sha256;
use std::collections::{BTreeMap, BTreeSet};

type R<T> = Result<T, Diagnostic>;

pub const MAX_PACKAGES: usize = 512;
pub const MAX_TREE_FILES: usize = 4096;
pub const MAX_LOCK_BYTES: usize = 256 * 1024;
const TREE_DOMAIN: &[u8] = b"atto-dependency-tree-v1\0";

fn err(code: &'static str, message: impl Into<String>) -> Diagnostic {
    Diagnostic::new(Pos { line: 1, col: 1 }, code, message)
}

fn name_ok(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 64
        && s.as_bytes()[0].is_ascii_lowercase()
        && s.bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'_' | b'-'))
}

fn version_ok(s: &str) -> bool {
    let parts: Vec<&str> = s.split('.').collect();
    parts.len() == 3
        && parts
            .iter()
            .all(|p| !p.is_empty() && p.len() <= 9 && p.bytes().all(|b| b.is_ascii_digit()) && (p == &"0" || !p.starts_with('0')))
}

fn tree_path_ok(path: &str) -> bool {
    !path.is_empty()
        && path.len() <= 255
        && path.is_ascii()
        && !path.starts_with('/')
        && path.split('/').all(|seg| {
            !seg.is_empty()
                && seg != "."
                && seg != ".."
                && seg.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.'))
        })
}

fn hex(bytes: &[u8; 32]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn sha_ok(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// Canonical digest of a package file tree. Paths must be canonical
/// relative POSIX paths, unique, with no `.`/`..`; order is irrelevant.
pub fn tree_digest(files: &[(&str, &[u8])]) -> R<String> {
    if files.is_empty() || files.len() > MAX_TREE_FILES {
        return Err(err("E_LOCK_TREE", "package tree must contain 1..=4096 files"));
    }
    let mut sorted: BTreeMap<&str, &[u8]> = BTreeMap::new();
    for (path, bytes) in files {
        if !tree_path_ok(path) {
            return Err(err("E_LOCK_TREE", format!("noncanonical package file path '{path}'")));
        }
        if sorted.insert(path, bytes).is_some() {
            return Err(err("E_LOCK_TREE", format!("duplicate package file path '{path}'")));
        }
    }
    let mut buf = Vec::with_capacity(64 + sorted.len() * 96);
    buf.extend_from_slice(TREE_DOMAIN);
    buf.extend_from_slice(&(sorted.len() as u64).to_le_bytes());
    for (path, bytes) in sorted {
        buf.extend_from_slice(&(path.len() as u64).to_le_bytes());
        buf.extend_from_slice(path.as_bytes());
        buf.extend_from_slice(&sha256::sha256(bytes));
    }
    Ok(hex(&sha256::sha256(&buf)))
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Source {
    /// Vendored tree at a canonical root-relative directory.
    Vendor(String),
    /// Registry coordinate `owner/name`; never fetched in offline mode.
    Registry(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LockedPackage {
    pub name: String,
    pub version: String,
    pub source: Source,
    pub tree_sha256: String,
    /// Sorted, unique `name@version` identities.
    pub deps: Vec<String>,
}

impl LockedPackage {
    pub fn id(&self) -> String {
        format!("{}@{}", self.name, self.version)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Lock {
    pub root: String,
    pub offline: bool,
    /// Sorted by name (one version per name).
    pub packages: Vec<LockedPackage>,
}

fn quoted(raw: &str, line: usize) -> R<String> {
    let v = raw.trim();
    if v.len() < 2 || !v.starts_with('"') || !v.ends_with('"') {
        return Err(err("E_LOCK", format!("line {line}: values must be quoted strings")));
    }
    let inner = &v[1..v.len() - 1];
    if inner.contains('"') || inner.contains('\\') || !inner.is_ascii() {
        return Err(err("E_LOCK", format!("line {line}: escapes and non-ASCII are not allowed")));
    }
    Ok(inner.to_string())
}

fn parse_id(id: &str) -> Option<(&str, &str)> {
    let (n, v) = id.split_once('@')?;
    (name_ok(n) && version_ok(v)).then_some((n, v))
}

/// Parse, validate and require canonical form.
pub fn parse(text: &str) -> R<Lock> {
    if text.len() > MAX_LOCK_BYTES || text.contains('\0') || text.contains('\r') {
        return Err(err("E_LOCK", "lockfile exceeds budget or contains NUL/CR"));
    }
    #[derive(PartialEq)]
    enum At {
        Start,
        Lock,
        Package,
    }
    let mut at = At::Start;
    let (mut version, mut root, mut offline) = (None, None, None);
    let mut packages: Vec<BTreeMap<String, String>> = Vec::new();
    for (i, raw) in text.lines().enumerate() {
        let n = i + 1;
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        match line {
            "[lock]" if at == At::Start => {
                at = At::Lock;
                continue;
            }
            "[[package]]" if at != At::Start => {
                at = At::Package;
                if packages.len() >= MAX_PACKAGES {
                    return Err(err("E_LOCK", "more than 512 locked packages"));
                }
                packages.push(BTreeMap::new());
                continue;
            }
            _ if line.starts_with('[') => {
                return Err(err("E_LOCK", format!("line {n}: unexpected section '{line}'")));
            }
            _ => {}
        }
        let (k, v) = line
            .split_once('=')
            .ok_or_else(|| err("E_LOCK", format!("line {n}: expected key = \"value\"")))?;
        let (k, v) = (k.trim(), quoted(v, n)?);
        match at {
            At::Start => return Err(err("E_LOCK", "lockfile must start with [lock]")),
            At::Lock => {
                let slot = match k {
                    "version" => &mut version,
                    "root" => &mut root,
                    "offline" => &mut offline,
                    _ => return Err(err("E_LOCK", format!("line {n}: unknown [lock] key '{k}'"))),
                };
                if slot.replace(v).is_some() {
                    return Err(err("E_LOCK", format!("line {n}: duplicate key '{k}'")));
                }
            }
            At::Package => {
                if !matches!(k, "name" | "version" | "source" | "tree_sha256" | "deps") {
                    return Err(err("E_LOCK", format!("line {n}: unknown package key '{k}'")));
                }
                if packages.last_mut().expect("package table").insert(k.to_string(), v).is_some() {
                    return Err(err("E_LOCK", format!("line {n}: duplicate key '{k}'")));
                }
            }
        }
    }
    if version.as_deref() != Some("1") {
        return Err(err("E_LOCK_VERSION", "unsupported or missing lock version (expected \"1\")"));
    }
    let root = root.ok_or_else(|| err("E_LOCK", "missing [lock].root"))?;
    if parse_id(&root).is_none() {
        return Err(err("E_LOCK", "[lock].root must be name@MAJOR.MINOR.PATCH"));
    }
    let offline = match offline.as_deref() {
        Some("true") => true,
        Some("false") | None => false,
        Some(_) => return Err(err("E_LOCK", "[lock].offline must be \"true\" or \"false\"")),
    };
    let mut out = Vec::with_capacity(packages.len());
    for table in packages {
        let get = |k: &str| table.get(k).cloned().ok_or_else(|| err("E_LOCK", format!("package missing '{k}'")));
        let (name, version, source, tree) = (get("name")?, get("version")?, get("source")?, get("tree_sha256")?);
        if !name_ok(&name) || !version_ok(&version) {
            return Err(err("E_LOCK", format!("invalid package identity '{name}@{version}'")));
        }
        if !sha_ok(&tree) {
            return Err(err("E_LOCK", "tree_sha256 must be 64 lowercase hex characters"));
        }
        let source = if let Some(dir) = source.strip_prefix("vendor:") {
            if !tree_path_ok(dir) {
                return Err(err("E_LOCK", "vendor source must be a canonical relative directory"));
            }
            Source::Vendor(dir.to_string())
        } else if let Some(coord) = source.strip_prefix("registry:") {
            let ok = coord.split_once('/').is_some_and(|(o, n)| name_ok(o) && name_ok(n));
            if !ok {
                return Err(err("E_LOCK", "registry source must be registry:owner/name"));
            }
            Source::Registry(coord.to_string())
        } else {
            return Err(err("E_LOCK", "source must be vendor:<dir> or registry:<owner>/<name>"));
        };
        let deps_raw = table.get("deps").cloned().unwrap_or_default();
        let deps: Vec<String> = if deps_raw.is_empty() {
            Vec::new()
        } else {
            deps_raw.split(',').map(str::to_string).collect()
        };
        out.push(LockedPackage {
            name,
            version,
            source,
            tree_sha256: tree,
            deps,
        });
    }
    let lock = Lock {
        root,
        offline,
        packages: out,
    };
    validate(&lock)?;
    if render(&lock) != text {
        return Err(err(
            "E_LOCK_CANONICAL",
            "lockfile is not in canonical form; regenerate it with the toolchain",
        ));
    }
    Ok(lock)
}

fn validate(lock: &Lock) -> R<()> {
    let mut names = BTreeSet::new();
    let mut ids = BTreeSet::new();
    for p in &lock.packages {
        if !names.insert(p.name.as_str()) {
            return Err(err(
                "E_LOCK_DUPLICATE",
                format!("package '{}' is locked at more than one version", p.name),
            ));
        }
        ids.insert(p.id());
    }
    for p in &lock.packages {
        let mut seen = BTreeSet::new();
        for d in &p.deps {
            if parse_id(d).is_none() {
                return Err(err("E_LOCK", format!("invalid dependency identity '{d}'")));
            }
            if !ids.contains(d) {
                return Err(err("E_LOCK_UNRESOLVED", format!("{} depends on unlocked '{d}'", p.id())));
            }
            if !seen.insert(d) {
                return Err(err("E_LOCK", format!("{} lists '{d}' twice", p.id())));
            }
        }
    }
    install_order(lock).map(|_| ())
}

/// Canonical rendering: packages sorted by name, deps sorted, fixed key
/// order, `\n` line endings, one trailing newline.
pub fn render(lock: &Lock) -> String {
    let mut packages = lock.packages.clone();
    packages.sort_by(|a, b| a.name.cmp(&b.name));
    let mut s = String::from("# atto.lock v1: generated; do not edit by hand\n[lock]\nversion = \"1\"\n");
    s.push_str(&format!("root = \"{}\"\noffline = \"{}\"\n", lock.root, lock.offline));
    for p in packages {
        let mut deps = p.deps.clone();
        deps.sort();
        let source = match &p.source {
            Source::Vendor(d) => format!("vendor:{d}"),
            Source::Registry(c) => format!("registry:{c}"),
        };
        s.push_str(&format!(
            "\n[[package]]\nname = \"{}\"\nversion = \"{}\"\nsource = \"{}\"\ntree_sha256 = \"{}\"\ndeps = \"{}\"\n",
            p.name,
            p.version,
            source,
            p.tree_sha256,
            deps.join(",")
        ));
    }
    s
}

/// Deterministic dependencies-first order (Kahn's algorithm, ties broken
/// by name). Fails closed on cycles.
pub fn install_order(lock: &Lock) -> R<Vec<String>> {
    let mut indegree: BTreeMap<String, usize> = BTreeMap::new();
    let mut dependents: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for p in &lock.packages {
        indegree.entry(p.id()).or_insert(0);
        for d in &p.deps {
            *indegree.entry(p.id()).or_insert(0) += 1;
            dependents.entry(d.clone()).or_default().push(p.id());
        }
    }
    let mut ready: BTreeSet<String> = indegree.iter().filter(|(_, n)| **n == 0).map(|(k, _)| k.clone()).collect();
    let mut order = Vec::with_capacity(indegree.len());
    while let Some(next) = ready.iter().next().cloned() {
        ready.remove(&next);
        for dep in dependents.get(&next).cloned().unwrap_or_default() {
            let n = indegree.get_mut(&dep).expect("known package");
            *n -= 1;
            if *n == 0 {
                ready.insert(dep);
            }
        }
        order.push(next);
    }
    if order.len() != indegree.len() {
        return Err(err("E_LOCK_CYCLE", "dependency cycle in lockfile"));
    }
    Ok(order)
}

/// A vendored tree the caller has read (e.g. from `vendor/<dir>`).
pub struct VendoredTree<'a> {
    pub dir: &'a str,
    pub files: Vec<(&'a str, &'a [u8])>,
}

/// Verify every locked package against the supplied vendored trees.
/// Offline mode requires all packages to be vendored; online mode still
/// verifies every vendored package it was given and reports registry
/// packages that must be fetched (and then verified by `tree_digest`).
pub fn verify(lock: &Lock, vendored: &[VendoredTree<'_>]) -> R<Vec<String>> {
    let mut by_dir: BTreeMap<&str, &VendoredTree<'_>> = BTreeMap::new();
    for v in vendored {
        if by_dir.insert(v.dir, v).is_some() {
            return Err(err("E_LOCK_TREE", format!("vendored directory '{}' supplied twice", v.dir)));
        }
    }
    let mut to_fetch = Vec::new();
    for p in &lock.packages {
        match &p.source {
            Source::Vendor(dir) => {
                let tree = by_dir
                    .get(dir.as_str())
                    .ok_or_else(|| err("E_LOCK_MISSING", format!("vendored tree for {} is missing", p.id())))?;
                if tree_digest(&tree.files)? != p.tree_sha256 {
                    return Err(err(
                        "E_LOCK_HASH",
                        format!("vendored tree for {} does not match tree_sha256", p.id()),
                    ));
                }
            }
            Source::Registry(_) if lock.offline => {
                return Err(err(
                    "E_LOCK_OFFLINE",
                    format!("{} is registry-only but the lock requires an offline build", p.id()),
                ));
            }
            Source::Registry(_) => to_fetch.push(p.id()),
        }
    }
    Ok(to_fetch)
}

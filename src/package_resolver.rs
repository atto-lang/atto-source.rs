//! Trusted host-side resolver for a single filesystem Atto package.
//!
//! The browser wasm32 compiler does not read the filesystem. This API is
//! intended for host compilers with a known local package root. It verifies
//! lexical paths, physical containment, symlink-free path components, and a
//! strict bounded subset of .atto.pkg.toml before using compile_linked().
//!
//! This is NOT a dependency registry resolver or a race-proof hostile-FS
//! sandbox. A caller must ensure the package tree is not concurrently
//! replaced while compiling; openat2/capability directory handles are
//! required before adversarial concurrently mutable trees are supported.
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use crate::diag::{Diagnostic, Pos};
use crate::linker::{self, LinkUnit};

type R<T> = Result<T, Diagnostic>;
const MAX_FILES: usize = 64;
const MAX_MANIFEST: usize = 8 * 1024;

fn error(code: &'static str, message: impl Into<String>) -> Diagnostic {
    Diagnostic::new(Pos { line: 1, col: 1 }, code, message)
}
fn io_error(what: &str, e: std::io::Error) -> Diagnostic {
    error("E_IMPORT_PATH", format!("{what}: {e}"))
}
fn package_segment(s: &str) -> bool {
    !s.is_empty() && s.len() <= 64 && s.is_ascii() &&
    s.as_bytes()[0].is_ascii_alphanumeric() &&
    s.bytes().all(|b| b.is_ascii_alphanumeric() ||
        matches!(b, b'_' | b'-' | b'.'))
}
fn version_ok(s: &str) -> bool {
    s.len() <= 48 && s.is_ascii() &&
    s.split('.').count() >= 3 && s.bytes().all(|b| b.is_ascii_alphanumeric() ||
        matches!(b, b'-' | b'.')) &&
    s.as_bytes().first().is_some_and(u8::is_ascii_digit)
}

/// Strict TOML subset: one [package] table with name/version string keys.
/// Unknown or duplicate keys, other tables, escaped strings and interpolation
/// are intentionally unsupported rather than silently ignored.
pub fn parse_package_identity(manifest: &str) -> R<String> {
    if manifest.len() > MAX_MANIFEST || manifest.contains('\0') {
        return Err(error("E_MANIFEST", "manifest exceeds budget or contains NUL"));
    }
    let mut in_package = false;
    let (mut name, mut version): (Option<String>, Option<String>) = (None, None);
    for (i, raw) in manifest.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') { continue; }
        if line == "[package]" {
            if in_package || name.is_some() || version.is_some() {
                return Err(error("E_MANIFEST", format!("duplicate package table at line {}", i+1)));
            }
            in_package = true;
            continue;
        }
        if line.starts_with('[') || !in_package {
            return Err(error("E_MANIFEST", format!("unsupported manifest table at line {}", i+1)));
        }
        let (key, value) = line.split_once('=').ok_or_else(||
            error("E_MANIFEST", format!("expected key=value at line {}", i+1)))?;
        let key = key.trim();
        let value = value.trim();
        let value = value.strip_prefix('"').ok_or_else(||
            error("E_MANIFEST", "package values must be quoted ASCII strings"))?;
        let end = value.find('"').ok_or_else(||
            error("E_MANIFEST", "unterminated package string"))?;
        let (raw_value, suffix) = value.split_at(end);
        if raw_value.contains('\\') || !raw_value.is_ascii() ||
           (!suffix[1..].trim().is_empty() && !suffix[1..].trim().starts_with('#')) {
            return Err(error("E_MANIFEST", "unsupported package value or inline content"));
        }
        match key {
            "name" if name.is_none() => name = Some(raw_value.to_string()),
            "version" if version.is_none() => version = Some(raw_value.to_string()),
            _ => return Err(error("E_MANIFEST", "unknown or duplicate package key")),
        }
    }
    let name = name.ok_or_else(|| error("E_MANIFEST", "missing [package].name"))?;
    let version = version.ok_or_else(|| error("E_MANIFEST", "missing [package].version"))?;
    if !package_segment(&name) || !version_ok(&version) {
        return Err(error("E_MANIFEST", "invalid package name or version"));
    }
    let identity = format!("{name}@{version}");
    if identity.len() > 128 {
        return Err(error("E_MANIFEST", "canonical package identity exceeds budget"));
    }
    Ok(identity)
}

/// Verify every physical path component and containment, not just string
/// prefixes. No source read is permitted through a symlink or outside root.
fn checked_path(root: &Path, relative: &str) -> R<PathBuf> {
    if !linker::path_ok(relative) {
        return Err(error("E_IMPORT_PATH", "noncanonical package source path"));
    }
    let mut candidate = root.to_path_buf();
    for component in relative.split('/') {
        candidate.push(component);
        let meta = fs::symlink_metadata(&candidate)
            .map_err(|e| io_error("missing package source component", e))?;
        if meta.file_type().is_symlink() {
            return Err(error("E_IMPORT_PATH", "package source path includes a symbolic link"));
        }
    }
    let physical = fs::canonicalize(&candidate)
        .map_err(|e| io_error("cannot canonicalize package source", e))?;
    if !physical.starts_with(root) || !physical.is_file() {
        return Err(error("E_IMPORT_PATH", "package source escapes root or is not a regular file"));
    }
    Ok(physical)
}

/// Read only the entry and reachable import graph, never scan unrelated
/// directories. Every path is validated against the canonical package root.
/// Cross-package dependencies require an authenticated dependency lock and
/// are deliberately not supported by this first host resolver.
pub fn compile_package(root: &Path, entry: &str) -> R<Vec<u8>> {
    if !linker::path_ok(entry) {
        return Err(error("E_IMPORT_PATH", "entry path must be canonical and package-relative"));
    }
    let root_meta = fs::symlink_metadata(root)
        .map_err(|e| io_error("cannot inspect package root", e))?;
    if root_meta.file_type().is_symlink() || !root_meta.is_dir() {
        return Err(error("E_IMPORT_PATH", "package root must be a directory, not a symlink"));
    }
    let canonical = fs::canonicalize(root)
        .map_err(|e| io_error("cannot canonicalize package root", e))?;
    let manifest_path = canonical.join(".atto.pkg.toml");
    let meta = fs::symlink_metadata(&manifest_path)
        .map_err(|e| io_error("missing .atto.pkg.toml", e))?;
    if !meta.is_file() || meta.file_type().is_symlink() ||
       meta.len() as usize > MAX_MANIFEST {
        return Err(error("E_MANIFEST", "manifest must be a bounded, regular non-symlink file"));
    }
    let manifest = fs::read_to_string(&manifest_path)
        .map_err(|e| io_error("cannot read .atto.pkg.toml", e))?;
    let identity = parse_package_identity(&manifest)?;
    let mut pending = vec![entry.to_string()];
    let mut loaded = BTreeMap::<String, String>::new();
    let mut seen = BTreeSet::new();
    while let Some(relative) = pending.pop() {
        if !seen.insert(relative.clone()) { continue; }
        if seen.len() > MAX_FILES {
            return Err(error("E_SOURCE_LIMIT", "too many reachable package source files"));
        }
        let physical = checked_path(&canonical, &relative)?;
        let size = fs::metadata(&physical)
            .map_err(|e| io_error("cannot inspect package source", e))?.len();
        if size > crate::SOURCE_CAP as u64 {
            return Err(error("E_SOURCE_LIMIT", "source file exceeds browser compiler limit"));
        }
        let src = fs::read_to_string(&physical)
            .map_err(|e| io_error("cannot read package source file", e))?;
        if src.len() > crate::SOURCE_CAP {
            return Err(error("E_SOURCE_LIMIT", "package source file is too large"));
        }
        for child in linker::import_paths(&src, &relative)? {
            pending.push(child);
        }
        loaded.insert(relative, src);
    }
    let units: Vec<LinkUnit<'_>> = loaded.iter().map(|(path, source)| LinkUnit {
        path, source, package: &identity,
    }).collect();
    linker::compile_linked(entry, &units)
}

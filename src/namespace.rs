//! Deterministic file-scope namespace headers.
//!
//! Folder names come only from a supplied *relative logical source path*;
//! never from host-specific absolute paths, runtime reflection, or the CWD.
//! This normalization is syntax-only: it does not make external imports linkable.
use std::path::{Component, Path};

use crate::diag::{Diagnostic, Pos};
use crate::lexer::{Tok, Token};
use crate::parser::is_reserved;

type R<T> = Result<T, Diagnostic>;
const MAX_NAMESPACE_DEPTH: usize = 32;

fn word(t: &[Token], i: usize, name: &str) -> bool {
    matches!(t.get(i).map(|x| &x.tok), Some(Tok::Ident(s)) if s == name)
}

fn error(pos: Pos, code: &'static str, message: impl Into<String>) -> Diagnostic {
    Diagnostic::new(pos, code, message)
}

fn is_header(t: &[Token], i: usize) -> bool {
    word(t, i, "use")
        && (word(t, i + 1, "namespace")
            || (word(t, i + 1, "default") && word(t, i + 2, "namespace")))
}

fn identifier(value: &str) -> bool {
    let mut chars = value.chars();
    matches!(chars.next(), Some(c) if c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
        && (!is_reserved(value) || value == "types")
}

fn folder_names(path: Option<&Path>, depth: usize, pos: Pos) -> R<Vec<String>> {
    let path = path.ok_or_else(|| error(pos, "E_NAMESPACE_PATH",
        "default namespace requires a package-relative source path; source-only compilation cannot infer a folder"))?;
    let mut parts = Vec::new();
    for component in path.components() {
        match component {
            Component::Normal(name) => {
                let name = name.to_str().ok_or_else(|| error(pos, "E_NAMESPACE_PATH",
                    "source path contains a non-UTF-8 component"))?;
                parts.push(name);
            }
            Component::CurDir => {}
            _ => return Err(error(pos, "E_NAMESPACE_PATH",
                "namespace paths must be package-relative without parent traversal")),
        }
    }
    if parts.last().is_none_or(|filename| !filename.ends_with(".atto")) {
        return Err(error(pos, "E_NAMESPACE_PATH",
            "namespace resolution requires a logical .atto source file"));
    }
    if parts.len() <= depth {
        return Err(error(pos, "E_NAMESPACE_DEPTH",
            format!("namespace depth {depth} exceeds available source directories")));
    }
    let names = &parts[parts.len() - depth - 1..parts.len() - 1];
    for name in names {
        if !identifier(name) {
            return Err(error(pos, "E_NAMESPACE_PATH",
                format!("folder name '{name}' is not a valid namespace identifier")));
        }
    }
    Ok(names.iter().map(|name| (*name).to_owned()).collect())
}

/// Validate and normalize a file-level namespace header. For the derived form,
/// replace it with the identical explicit namespace header that the target
/// parser already understands. Keep the caller's original token positions.
pub fn normalize(t: &[Token], logical_source: Option<&Path>) -> R<Vec<Token>> {
    if t.is_empty() {
        return Ok(Vec::new());
    }
    let mut output = t.to_vec();
    let mut seen = false;
    let mut braces = 0usize;
    let mut i = 0usize;
    while i < t.len() {
        if braces == 0 && is_header(t, i) {
            let pos = t[i].pos;
            if seen {
                return Err(error(pos, "E_DUPLICATE",
                    "only one namespace declaration is allowed per source file"));
            }
            if i != 0 {
                return Err(error(pos, "E_SYNTAX",
                    "namespace declaration must precede all imports and declarations"));
            }
            seen = true;
            let derived = word(t, i + 1, "default");
            let mut cursor = i + if derived { 3 } else { 2 };
            let names = if derived {
                let mut depth = 1usize;
                if matches!(t.get(cursor).map(|x| &x.tok), Some(Tok::LParen)) {
                    cursor += 1;
                    match t.get(cursor).map(|x| &x.tok) {
                        Some(Tok::Int(Some(value)))
                            if *value > 0 && *value <= MAX_NAMESPACE_DEPTH as u64 => {
                                depth = *value as usize;
                            }
                        _ => return Err(error(pos, "E_NAMESPACE_DEPTH",
                            "namespace depth must be an integer between 1 and 32")),
                    }
                    cursor += 1;
                    if !matches!(t.get(cursor).map(|x| &x.tok), Some(Tok::RParen)) {
                        return Err(error(pos, "E_SYNTAX",
                            "expected ')' after namespace depth"));
                    }
                    cursor += 1;
                }
                folder_names(logical_source, depth, pos)?
            } else {
                let mut names = Vec::new();
                loop {
                    match t.get(cursor).map(|x| &x.tok) {
                        Some(Tok::Ident(name)) if identifier(name) => names.push(name.clone()),
                        _ => return Err(error(pos, "E_SYNTAX",
                            "namespace requires a valid identifier")),
                    }
                    cursor += 1;
                    if !matches!(t.get(cursor).map(|x| &x.tok), Some(Tok::Dot)) {
                        break;
                    }
                    cursor += 1;
                    if names.len() >= MAX_NAMESPACE_DEPTH {
                        return Err(error(pos, "E_NAMESPACE_DEPTH",
                            "namespace has more than 32 components"));
                    }
                }
                names
            };
            if !matches!(t.get(cursor).map(|x| &x.tok), Some(Tok::Semi)) {
                return Err(error(pos, "E_SYNTAX", "expected ';' after namespace declaration"));
            }
            cursor += 1;
            let mut header = vec![
                Token { tok: Tok::Ident("use".into()), pos },
                Token { tok: Tok::Ident("namespace".into()), pos },
            ];
            for (j, name) in names.into_iter().enumerate() {
                if j != 0 {
                    header.push(Token { tok: Tok::Dot, pos });
                }
                header.push(Token { tok: Tok::Ident(name), pos });
            }
            header.push(Token { tok: Tok::Semi, pos });
            output.splice(i..cursor, header);
            i = cursor;
            continue;
        }
        match t[i].tok {
            Tok::LBrace => braces += 1,
            Tok::RBrace => braces = braces.saturating_sub(1),
            _ => {}
        }
        i += 1;
    }
    Ok(output)
}

/// Tagged scope selection for the new namespace profile. This is syntax and
/// logical-path normalization only. The package linker must supply the
/// authenticated package and source-unit identity; no attachment authority is
/// inferred from a path. The legacy normalize/compile APIs are unchanged.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NamespaceModeV2 {
    File,
    Named(Vec<String>),
    Folder(Vec<String>),
}

/// Validate a canonical relative source ID without consulting the filesystem.
/// Folder identity retains every parent component, including non-identifier
/// names. Numeric default names still use the identifier-checked legacy parser.
fn logical_folder_v2(path: Option<&Path>, pos: Pos) -> R<Vec<String>> {
    let path = path.and_then(Path::to_str).ok_or_else(|| error(pos, "E_NAMESPACE_PATH",
        "folder namespace requires a verified logical source path"))?;
    if path.contains('\\') || path.contains(':') || path.contains('\0') {
        return Err(error(pos, "E_NAMESPACE_PATH", "invalid logical source path"));
    }
    let mut parts: Vec<_> = path.split('/').collect();
    if parts.iter().any(|p| p.is_empty() || *p == "." || *p == "..") ||
        parts.last().is_none_or(|p| !p.ends_with(".atto") || p.len() <= 5) {
        return Err(error(pos, "E_NAMESPACE_PATH", "invalid logical source path"));
    }
    parts.pop();
    Ok(parts.into_iter().map(str::to_owned).collect())
}

fn header_v2(t: &[Token], i: usize) -> bool {
    is_header(t, i) || (word(t, i, "use")
        && (word(t, i + 1, "file") || word(t, i + 1, "folder"))
        && word(t, i + 2, "namespace"))
}

/// Resolve v2 header identity while leaving production legacy normalization
/// untouched until the edition selector and tagged package linker land.
/// An explicit numeric default is a NAMED scope, not a FOLDER scope.
/// A file-local mode is not a globally unique file ID: callers must bind it
/// to the authenticated source-unit identity before cross-file use.
pub fn resolve_mode_v2(t: &[Token], logical_source: Option<&Path>) -> R<NamespaceModeV2> {
    let mut seen = false;
    let mut braces = 0usize;
    for i in 0..t.len() {
        if braces == 0 && header_v2(t, i) {
            if seen {
                return Err(error(t[i].pos, "E_DUPLICATE",
                    "only one namespace declaration is allowed per source file"));
            }
            if i != 0 {
                return Err(error(t[i].pos, "E_SYNTAX",
                    "namespace declaration must precede all imports and declarations"));
            }
            seen = true;
        }
        match t[i].tok {
            Tok::LBrace => braces += 1,
            Tok::RBrace => braces = braces.saturating_sub(1),
            _ => {}
        }
    }
    if !seen {
        return Ok(NamespaceModeV2::File);
    }
    let pos = t[0].pos;
    let bare_default = word(t, 1, "default")
        && !matches!(t.get(3).map(|x| &x.tok), Some(Tok::LParen));
    if bare_default || word(t, 1, "file") || word(t, 1, "folder") {
        if !matches!(t.get(3).map(|x| &x.tok), Some(Tok::Semi)) {
            return Err(error(pos, "E_SYNTAX", "expected ';' after namespace declaration"));
        }
        return if word(t, 1, "folder") {
            Ok(NamespaceModeV2::Folder(logical_folder_v2(logical_source, pos)?))
        } else {
            Ok(NamespaceModeV2::File)
        };
    }
    if word(t, 1, "default") {
        // Reject noncanonical path spellings before the legacy numeric
        // normalizer (which tolerates CurDir) can discard them.
        logical_folder_v2(logical_source, pos)?;
    }
    let normalized = normalize(t, logical_source)?;
    let names: Vec<String> = normalized.iter().skip(2).take_while(|x| x.tok != Tok::Semi)
        .filter_map(|x| match &x.tok {
            Tok::Ident(name) => Some(name.clone()),
            _ => None,
        }).collect();
    if names.iter().any(|n| n == "parent" || n == "super" || n == "self" || n == "pkg"
        || !(n == "types" || crate::static_import_graph::valid_component(n))) {
        return Err(error(pos, if word(t, 1, "default") { "E_NAMESPACE_PATH" } else { "E_SYNTAX" },
            "invalid v2 namespace component"));
    }
    Ok(NamespaceModeV2::Named(names))
}

/// Scope identity within an authenticated package snapshot. File mode is never
/// a shared singleton; the validated source-unit path is part of its key.
/// The PackageId's authentication remains the caller's responsibility.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ScopeIdentityV2 {
    File(String),
    Named(Vec<String>),
    Folder(Vec<String>),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScopeKeyV2 {
    package: crate::static_import_graph::PackageId,
    identity: ScopeIdentityV2,
}

pub fn resolve_scope_key_v2(
    t: &[Token],
    logical_source: &Path,
    package: &crate::static_import_graph::PackageId,
) -> R<ScopeKeyV2> {
    logical_folder_v2(Some(logical_source), Pos::default())?;
    let identity = match resolve_mode_v2(t, Some(logical_source))? {
        NamespaceModeV2::File => ScopeIdentityV2::File(
            logical_source.to_str().ok_or_else(|| error(Pos::default(), "E_NAMESPACE_PATH",
                "source identity is not UTF-8"))?.to_owned()),
        NamespaceModeV2::Named(names) => ScopeIdentityV2::Named(names),
        NamespaceModeV2::Folder(parts) => ScopeIdentityV2::Folder(parts),
    };
    Ok(ScopeKeyV2 { package: package.clone(), identity })
}

/// Prepare a single-file v2 input for the existing, full compiler pipeline.
/// No shared namespace is lowered as a file-local scope: those modes require
/// the authenticated package linker. Mask only a lexer-validated first header
/// and preserve source line/column positions for downstream diagnostics.
pub(crate) fn prepare_file_v2(source: &str) -> R<String> {
    let pos = Pos { line: 1, col: 1 };
    if source.len() > crate::SOURCE_CAP {
        return Err(error(pos, "E_SOURCE_LIMIT", "source exceeds compiler byte limit"));
    }
    let tokens = crate::lexer::lex(source)?;
    if resolve_mode_v2(&tokens, None)? != NamespaceModeV2::File {
        return Err(error(pos, "E_UNSUPPORTED",
            "shared namespace v2 requires authenticated package linking"));
    }
    if !header_v2(&tokens, 0) {
        return Ok(source.to_owned());
    }
    // File/default headers have exactly four tokens. resolve_mode_v2
    // already checked the semicolon, first position and header uniqueness.
    let start = tokens[0].pos;
    let end = tokens[3].pos;
    let mut result = String::with_capacity(source.len());
    let (mut line, mut col) = (1u32, 1u32);
    let mut masking = false;
    let mut ended = false;
    for ch in source.chars() {
        if (line, col) == (start.line, start.col) { masking = true; }
        if masking && !ended {
            result.push(if ch == '\n' || ch == '\r' { ch } else { ' ' });
            if (line, col) == (end.line, end.col) {
                if ch != ';' { return Err(error(end, "E_SYNTAX", "invalid namespace span")); }
                ended = true;
            }
        } else {
            result.push(ch);
        }
        if ch == '\n' { line += 1; col = 1; } else { col += 1; }
    }
    if !ended { return Err(error(end, "E_SYNTAX", "unterminated namespace span")); }
    Ok(result)
}

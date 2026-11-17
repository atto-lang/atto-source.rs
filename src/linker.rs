//! Bounded, in-memory single-hop namespace/module linking for the browser guest.
//!
//! This is a deterministic static source linker, NOT an on-disk package resolver.
//! The caller must provide source units and trusted canonical package identities;
//! no symlinks, network fetches or host capabilities are consulted. Reject
//! unsupported transitive imports and untrusted re-exports before emitting Wasm.
use std::collections::{BTreeMap, BTreeSet};
use crate::diag::{Diagnostic, Pos};
use crate::lexer::{self, Tok, Token};
use crate::{module_lower, parser, visibility_lower};

type R<T> = Result<T, Diagnostic>;
const MAX_UNITS: usize = 64;
const MAX_IMPORTS: usize = 32;

#[derive(Clone, Copy, Debug)]
pub struct LinkUnit<'a> {
    /// Normalized, project-relative POSIX path such as "src/main.atto".
    pub path: &'a str,
    /// Canonical package identity supplied by an already trusted resolver.
    pub package: &'a str,
    pub source: &'a str,
}

#[derive(Clone, Debug)]
struct Import {
    name: String,
    path: String,
    pos: Pos,
}

struct Header {
    namespace: Option<String>,
    imports: Vec<Import>,
    body: String,
}

fn err(pos: Pos, code: &'static str, message: impl Into<String>) -> Diagnostic {
    Diagnostic::new(pos, code, message)
}

fn word(t: &[Token], i: usize, val: &str) -> bool {
    matches!(t.get(i).map(|x| &x.tok), Some(Tok::Ident(s)) if s == val)
}
fn ident(t: &[Token], i: usize) -> R<String> {
    match t.get(i) {
        Some(Token { tok: Tok::Ident(s), pos }) if
            s.len() <= 64 && !s.is_empty() && s.is_ascii() &&
            (s.as_bytes()[0].is_ascii_alphabetic() || s.starts_with('_')) &&
            s.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_') =>
            Ok(s.clone()),
        Some(t) => Err(err(t.pos, "E_SYNTAX", "expected bounded ASCII namespace/module identifier")),
        None => Err(err(Pos::default(), "E_EOF", "expected identifier")),
    }
}

/// Token positions are Unicode scalar columns, not bytes. Calculate the
/// original byte offset explicitly; never byte-slice using a column directly.
fn byte_at(source: &str, pos: Pos) -> R<usize> {
    let (mut line, mut col) = (1u32, 1u32);
    for (offset, c) in source.char_indices() {
        if line == pos.line && col == pos.col { return Ok(offset); }
        if c == '\n' { line += 1; col = 1; } else { col += 1; }
    }
    if line == pos.line && col == pos.col { return Ok(source.len()); }
    Err(err(pos, "E_SYNTAX", "invalid source token boundary"))
}
fn erase(mut source: String, mut spans: Vec<(usize, usize)>) -> String {
    spans.sort_unstable_by(|a, b| b.0.cmp(&a.0));
    for (start, end) in spans { source.replace_range(start..end, ""); }
    source
}
fn span(source: &str, toks: &[Token], start: usize, semi: usize) -> R<(usize, usize)> {
    if toks.get(semi).map(|t| &t.tok) != Some(&Tok::Semi) {
        return Err(err(toks[start].pos, "E_SYNTAX", "missing semicolon"));
    }
    Ok((byte_at(source, toks[start].pos)?, byte_at(source, toks[semi].pos)? + 1))
}

fn path_ok(path: &str) -> bool {
    path.len() <= 256 && !path.is_empty() && path.is_ascii() && path.ends_with(".atto") &&
        !path.starts_with('/') && !path.contains('\\') && !path.contains(':') &&
        path.split('/').all(|part| !part.is_empty() && part != "." && part != ".." &&
            part.bytes().all(|b| b.is_ascii_alphanumeric() ||
                matches!(b, b'_' | b'-' | b'.')))
}
fn package_ok(package: &str) -> bool {
    !package.is_empty() && package.len() <= 128 && package.is_ascii() &&
        package.bytes().all(|b| b.is_ascii_alphanumeric() ||
            matches!(b, b'_' | b'-' | b'.' | b'@'))
}
fn relative_path(from: &str, rel: &str, pos: Pos) -> R<String> {
    if !rel.starts_with("./") || !path_ok(&rel[2..]) {
        return Err(err(pos, "E_IMPORT_PATH",
            "imports must use canonical './name.atto' relative paths without traversal"));
    }
    let dir = from.rsplit_once('/').map(|(dir, _)| dir).unwrap_or("");
    let resolved = if dir.is_empty() { rel[2..].to_string() }
                   else { format!("{dir}/{}", &rel[2..]) };
    if !path_ok(&resolved) {
        return Err(err(pos, "E_IMPORT_PATH", "invalid canonical import destination"));
    }
    Ok(resolved)
}

/// Consume only a first-position namespace header and following import module
/// declarations. No import is executable merely because its name occurs in a
/// namespace. The path must resolve exactly among provided input units.
fn header(source: &str, file: &str) -> R<Header> {
    let t = lexer::lex(source)?;
    let mut i = 0;
    let mut namespace = None;
    let mut imports = Vec::new();
    let mut spans = Vec::new();
    if word(&t, i, "use") {
        if !word(&t, i + 1, "namespace") || t.get(i + 3).map(|x| &x.tok) != Some(&Tok::Semi) {
            return Err(err(t[i].pos, "E_SYNTAX", "expected 'use namespace Name;' as first declaration"));
        }
        namespace = Some(ident(&t, i + 2)?);
        spans.push(span(source, &t, i, i + 3)?);
        i += 4;
    }
    while word(&t, i, "import") {
        let pos = t[i].pos;
        if !word(&t, i + 1, "module") || !word(&t, i + 3, "from") {
            return Err(err(pos, "E_SYNTAX", "expected 'import module Name from \"./file.atto\";'"));
        }
        let name = ident(&t, i + 2)?;
        let path = match t.get(i + 4).map(|x| &x.tok) {
            Some(Tok::Str(s)) => relative_path(file, s, pos)?,
            _ => return Err(err(pos, "E_SYNTAX", "module import requires quoted relative path")),
        };
        if t.get(i + 5).map(|x| &x.tok) != Some(&Tok::Semi) {
            return Err(err(pos, "E_SYNTAX", "module import must end with a semicolon"));
        }
        if imports.len() >= MAX_IMPORTS {
            return Err(err(pos, "E_SOURCE_LIMIT", "too many module imports"));
        }
        if imports.iter().any(|x: &Import| x.name == name || x.path == path) {
            return Err(err(pos, "E_DUPLICATE", "duplicate imported module alias or path"));
        }
        imports.push(Import { name, path, pos });
        spans.push(span(source, &t, i, i + 5)?);
        i += 6;
    }
    // Both forms are declarations, not expressions. A second header or
    // import (including an attempted nested one) must never be silently lost.
    for token in &t[i..] {
        if let Tok::Ident(s) = &token.tok {
            if s == "import" || s == "namespace" || s == "use" {
                return Err(err(token.pos, "E_SYNTAX",
                    "namespace headers and module imports must precede all declarations"));
            }
        }
    }
    Ok(Header { namespace, imports, body: erase(source.to_string(), spans) })
}

fn imported_module(imp: &Import, dependency: &LinkUnit<'_>,
                   entry_package: &str) -> R<String> {
    let hdr = header(dependency.source, dependency.path)?;
    if !hdr.imports.is_empty() {
        if hdr.imports.iter().any(|child| child.path == imp.path) {
            return Err(err(imp.pos, "E_IMPORT_CYCLE", "module imports cannot cycle"));
        }
        return Err(err(imp.pos, "E_UNSUPPORTED",
            "transitive module import linking is not yet supported"));
    }
    let tokens = lexer::lex(&hdr.body)?;
    // Only a real, file-owned module declaration may satisfy an import.
    let mut visible = visibility_lower::lower(tokens.clone())?;
    let lowered = module_lower::lower_with_manifest(visible.tokens)?;
    if lowered.modules.len() != 1 || lowered.modules[0].name != imp.name {
        return Err(err(imp.pos, "E_UNBOUND",
            format!("import '{}' does not match exactly one declared module in its source", imp.name)));
    }
    let parsed = parser::Parser::new(lowered.tokens).parse_program()?;
    if !parsed.classes.is_empty() || !parsed.actors.is_empty() || !parsed.enums.is_empty() ||
       parsed.funcs.iter().any(|f| !f.name.starts_with("__atto_internal_module_")) {
        return Err(err(imp.pos, "E_UNSUPPORTED",
            "imported source must contain only the requested module and its functions"));
    }
    if visible.exports.iter().any(|e| e.name != imp.name) {
        return Err(err(imp.pos, "E_UNSUPPORTED",
            "imported module source cannot expose unrelated file-level symbols"));
    }
    visibility_lower::resolve_exports(&mut visible.exports, &parsed, &lowered.modules)?;
    let module = &lowered.modules[0];
    let allowed = visible.exports.iter().find(|e| e.name == imp.name);
    let exported = allowed.is_some() || module.inline_public;
    if !exported {
        return Err(err(imp.pos, "E_VISIBILITY", "imported module is not exported"));
    }
    let public = module.inline_public || allowed.is_some_and(|e|
        e.visibility == visibility_lower::Visibility::Public);
    if !public && dependency.package != entry_package {
        return Err(err(imp.pos, "E_VISIBILITY",
            "pub[pkg] module cannot be imported by a different package"));
    }
    // Erase all file export directives: a referenced module's visibility is
    // checked above, but must NOT be serialized as an entry-file re-export.
    let mut spans = Vec::new();
    let mut i = 0;
    while i < tokens.len() {
        if word(&tokens, i, "export") {
            let (_, next) = visibility_lower::parse_export_directive(&tokens, i)?;
            spans.push(span(&hdr.body, &tokens, i, next - 1)?);
            i = next;
            continue;
        }
        if word(&tokens, i, "internal") || word(&tokens, i, "external") ||
            word(&tokens, i, "protected") {
            return Err(err(tokens[i].pos, "E_UNSUPPORTED",
                "imported module cannot contain unresolved local/foreign/protected modifiers"));
        }
        // An imported inline-public module is made file-local in the
        // combined guest, so it cannot become an accidental re-export.
        if word(&tokens, i, "define") && word(&tokens, i + 1, "pub") &&
           word(&tokens, i + 2, "module") {
            let start = byte_at(&hdr.body, tokens[i + 1].pos)?;
            spans.push((start, start + 3)); // exact ASCII 'pub'
            i += 3;
            continue;
        }
        i += 1;
    }
    Ok(erase(hdr.body, spans))
}

/// Compile a bounded, explicit source graph into one guest Wasm module.
/// Exactly one entry file and at most one import hop. Package identities
/// MUST originate from a trusted package resolver, not self-asserted
/// untrusted source attributes. No filesystem, network or symlink traversal.
pub fn compile_linked(entry_path: &str, units: &[LinkUnit<'_>]) -> R<Vec<u8>> {
    if units.is_empty() || units.len() > MAX_UNITS {
        return Err(err(Pos::default(), "E_SOURCE_LIMIT", "invalid linked source-unit count"));
    }
    let mut files = BTreeMap::new();
    for unit in units {
        if !path_ok(unit.path) || !package_ok(unit.package) {
            return Err(err(Pos::default(), "E_IMPORT_PATH",
                "source path or package identity is not canonical"));
        }
        if files.insert(unit.path, unit).is_some() {
            return Err(err(Pos::default(), "E_DUPLICATE", "duplicate canonical source file path"));
        }
    }
    let entry = files.get(entry_path).ok_or_else(|| err(Pos::default(), "E_UNBOUND",
        "entry source file does not exist in the explicitly supplied unit set"))?;
    let root = header(entry.source, entry_path)?;
    let mut pieces = Vec::new();
    let mut seen = BTreeSet::new();
    for imp in &root.imports {
        if imp.path == entry_path {
            return Err(err(imp.pos, "E_IMPORT_CYCLE", "module cannot import itself"));
        }
        if !seen.insert(imp.name.as_str()) {
            return Err(err(imp.pos, "E_DUPLICATE", "imported module alias is duplicated"));
        }
        let dep = files.get(imp.path.as_str()).ok_or_else(|| err(imp.pos,
            "E_UNBOUND", format!("missing imported file '{}'", imp.path)))?;
        pieces.push(imported_module(imp, dep, entry.package)?);
    }
    // Re-exporting an imported module is not equivalent to declaring it
    // locally. This explicit denial is independent of the flattening pass.
    let own_exports = visibility_lower::lower(lexer::lex(&root.body)?)?;
    for export in own_exports.exports {
        if seen.contains(export.name.as_str()) {
            return Err(err(export.pos, "E_VISIBILITY",
                "imported modules are not implicitly or explicitly re-exportable in linker v1"));
        }
    }
    pieces.push(root.body);
    let linked = pieces.join("\n");
    if linked.len() > crate::SOURCE_CAP {
        return Err(err(Pos::default(), "E_SOURCE_LIMIT", "linked guest source exceeds browser limit"));
    }
    // Namespace headers establish ownership only; they never open imported
    // packages for unqualified name resolution. Exactly named imports are
    // flattened with their verified visibility above.
    let _entry_namespace = root.namespace;
    crate::compile(&linked)
}

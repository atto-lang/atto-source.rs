//! Compiler-owned `core`, `alloc` and `std` library units
//! (atto-spec language/core-alloc-std-libraries-v1.md).
//!
//! `import module num from "core/num";` resolves to source embedded in this
//! compiler, never to the filesystem or a package. Each unit is one
//! `define module <unit> as ... end`. Expansion blanks the import statement
//! (keeping line/column positions of the user's code) and appends the unit
//! sources, dependencies first, so the ordinary single-file pipeline
//! (module_lower, parser, checker, Wasm emitter) compiles them.
use crate::diag::{Diagnostic, Pos};
use crate::lexer::{self, Tok, Token};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::{AtomicU8, Ordering};

type R<T> = Result<T, Diagnostic>;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Layer {
    Core,
    Alloc,
    Std,
}

impl Layer {
    pub fn name(self) -> &'static str {
        match self {
            Layer::Core => "atto.core",
            Layer::Alloc => "atto.alloc",
            Layer::Std => "atto.std",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TargetOs {
    Posix,
    Windows,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum OsFilter {
    Any,
    Posix,
    Windows,
}

struct Unit {
    layer: Layer,
    name: &'static str,
    os: OsFilter,
    source: &'static str,
}

const UNITS: &[Unit] = &[
    Unit { layer: Layer::Core, name: "num", os: OsFilter::Any, source: include_str!("../stdlib/core/num.atto") },
    Unit { layer: Layer::Core, name: "cmp", os: OsFilter::Any, source: include_str!("../stdlib/core/cmp.atto") },
    Unit { layer: Layer::Core, name: "option", os: OsFilter::Any, source: include_str!("../stdlib/core/option.atto") },
    Unit { layer: Layer::Core, name: "result", os: OsFilter::Any, source: include_str!("../stdlib/core/result.atto") },
    Unit { layer: Layer::Core, name: "mem", os: OsFilter::Any, source: include_str!("../stdlib/core/mem.atto") },
    Unit { layer: Layer::Alloc, name: "arena", os: OsFilter::Any, source: include_str!("../stdlib/alloc/arena.atto") },
    Unit { layer: Layer::Std, name: "io", os: OsFilter::Any, source: include_str!("../stdlib/std/io.atto") },
    Unit { layer: Layer::Std, name: "path", os: OsFilter::Posix, source: include_str!("../stdlib/std/path_posix.atto") },
    Unit { layer: Layer::Std, name: "path", os: OsFilter::Windows, source: include_str!("../stdlib/std/path_windows.atto") },
];

const MAX_UNITS: usize = 32;

static TARGET_OS: AtomicU8 = AtomicU8::new(if cfg!(windows) { 2 } else { 1 });

/// Select the OS family for platform-variant units such as `std/path`.
pub fn set_target_os(os: TargetOs) {
    TARGET_OS.store(if os == TargetOs::Windows { 2 } else { 1 }, Ordering::Relaxed);
}

pub fn target_os() -> TargetOs {
    if TARGET_OS.load(Ordering::Relaxed) == 2 { TargetOs::Windows } else { TargetOs::Posix }
}

fn err(pos: Pos, code: &'static str, msg: impl Into<String>) -> Diagnostic {
    Diagnostic::new(pos, code, msg)
}

/// True for any selector under a reserved library root.
pub fn is_reserved(selector: &str) -> bool {
    ["core", "alloc", "std", "atto.core", "atto.alloc", "atto.std", "ores.core", "ores.std"]
        .iter()
        .any(|root| selector == *root || selector.starts_with(&format!("{root}/"))
            || (matches!(*root, "core" | "alloc" | "std") && selector.starts_with(&format!("{root}."))))
}

/// Canonical `<layer>.<unit>` and legacy quoted `atto.<layer>/<unit>` /
/// `<layer>/<unit>` (optional legacy `.atto` suffix). Units are flat. No `ores.*`.
pub fn normalize(selector: &str) -> Option<(Layer, &str)> {
    // AttoLang names only: ores.* is reserved (fails closed) but never a selector.
    const PREFIXES: &[(&str, Layer)] = &[
        ("atto.core/", Layer::Core), ("core/", Layer::Core),
        ("atto.alloc/", Layer::Alloc), ("alloc/", Layer::Alloc),
        ("atto.std/", Layer::Std), ("std/", Layer::Std),
        ("core.", Layer::Core), ("alloc.", Layer::Alloc), ("std.", Layer::Std),
    ];
    for (prefix, layer) in PREFIXES {
        if let Some(rest) = selector.strip_prefix(prefix) {
            let unit = if prefix.ends_with('.') { rest } else { rest.strip_suffix(".atto").unwrap_or(rest) };
            let ok = !unit.is_empty()
                && unit.len() <= 64
                && unit.as_bytes()[0].is_ascii_lowercase()
                && unit.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_');
            return ok.then_some((*layer, unit));
        }
    }
    None
}

fn find(layer: Layer, unit: &str, os: TargetOs) -> Option<&'static Unit> {
    UNITS.iter().find(|u| {
        u.layer == layer
            && u.name == unit
            && match u.os {
                OsFilter::Any => true,
                OsFilter::Posix => os == TargetOs::Posix,
                OsFilter::Windows => os == TargetOs::Windows,
            }
    })
}

struct LibImport {
    binding: String,
    aliases: Vec<String>,
    v2: bool,
    selector: String,
    pos: Pos,
    start: usize,
    end: usize,
}

fn byte_offset(source: &str, pos: Pos) -> usize {
    let (mut line, mut col) = (1u32, 1u32);
    for (offset, c) in source.char_indices() {
        if line == pos.line && col == pos.col {
            return offset;
        }
        if c == '\n' {
            line += 1;
            col = 1;
        } else {
            col += 1;
        }
    }
    source.len()
}

fn ident(t: Option<&Token>) -> Option<&str> {
    match t.map(|t| &t.tok) {
        Some(Tok::Ident(s)) => Some(s.as_str()),
        _ => None,
    }
}

fn binding_path(tokens: &[Token], pos: Pos) -> R<String> {
    if tokens.is_empty() || tokens.len() > 15 || tokens.len() % 2 == 0 {
        return Err(err(pos, "E_IMPORT", "invalid bounded import path"));
    }
    let mut parts = Vec::new();
    for (i, token) in tokens.iter().enumerate() {
        if i % 2 == 1 {
            if token.tok != Tok::Dot { return Err(err(token.pos, "E_IMPORT", "expected dot in import path")); }
        } else if let Some(name) = ident(Some(token)) {
            if crate::parser::is_reserved(name) || matches!(name, "pkg" | "enum" | "use" | "declare" | "mtd") || name.len() > 64
                || name.starts_with("__atto_") {
                return Err(err(token.pos, "E_IMPORT", "invalid import binding identifier"));
            }
            parts.push(name);
        } else { return Err(err(token.pos, "E_IMPORT", "expected import identifier")); }
    }
    Ok(parts.join("."))
}

/// Shared parser for one complete builtin import clause, excluding semicolon.
/// No loading, registry lookup or binding publication occurs here.
fn parse_builtin_clause(body: &[Token]) -> R<Option<LibImport>> {
    let pos = body.first().map_or(Pos::default(), |t| t.pos);
    if ident(body.first()) != Some("import") { return Ok(None); }
    // Inspect only the source after 'from', never an arbitrary string
    // elsewhere in the clause. Recognize malformed reserved bare paths
    // too, so they cannot fall through to a file loader.
    let source_at = body.iter().position(|t| matches!(&t.tok, Tok::Ident(s) if s == "from"))
        .map(|at| at + 1);
    let mut selector = None;
    let mut requested = None;
    let mut direct = false;
    let mut explicit_alias = false;
    let mut legacy_module = false;
    let mut selected = None;
    if let Some(at) = source_at {
        let explicit_package = ident(body.get(at)) == Some("pkg");
        let source = if explicit_package { &body[at + 1..] } else { &body[at..] };
        let quoted = match source.first().map(|t| &t.tok) {
            Some(Tok::Str(s)) => Some(s.clone()),
            _ => None,
        };
        let bare_reserved = matches!(ident(source.first()), Some("core" | "alloc" | "std"));
        if explicit_package {
            if quoted.as_deref().is_some_and(is_reserved) || bare_reserved {
                return Err(err(pos, "E_UNSUPPORTED", "explicit package selector cannot use compiler-owned library resolution"));
            }
        } else if quoted.as_deref().is_some_and(is_reserved) {
            if source.len() != 1 {
                return Err(err(pos, "E_IMPORT", "malformed builtin selector"));
            }
            selector = quoted;
        } else if bare_reserved {
            if source.len() != 3 || source[1].tok != Tok::Dot || ident(source.get(2)).is_none() {
                return Err(err(pos, "E_IMPORT", "builtin selector must be a flat dotted identity"));
            }
            selector = Some(format!("{}.{}", ident(source.first()).unwrap(), ident(source.get(2)).unwrap()));
        }
        if selector.is_some() {
            let mut left = &body[1..at - 1];
            legacy_module = ident(left.first()) == Some("module");
            if legacy_module || ident(left.first()) == Some("namespace") { left = &left[1..]; }
            if let Some(at) = left.iter().position(|t| ident(Some(t)) == Some("as")) {
                if legacy_module { return Err(err(pos, "E_IMPORT", "legacy module import does not accept aliases")); }
                selected = Some(binding_path(&left[..at], pos)?);
                requested = Some(binding_path(&left[at + 1..], pos)?);
                explicit_alias = true;
            } else {
                requested = Some(binding_path(left, pos)?);
            }
        }
    } else {
        let mut left = &body[1..];
        if ident(left.first()) == Some("namespace") { left = &left[1..]; }
        if matches!(ident(left.first()), Some("core" | "alloc" | "std")) {
            direct = true;
            let as_at = left.iter().position(|t| matches!(&t.tok, Tok::Ident(s) if s == "as"));
            let source = &left[..as_at.unwrap_or(left.len())];
            selector = Some(binding_path(source, pos)?);
            if let Some(at) = as_at {
                explicit_alias = true;
                requested = Some(binding_path(&left[at + 1..], pos)?);
            } else if ident(body.get(1)) == Some("namespace") {
                requested = selector.clone();
            }
        }
    }
    if let Some(selector) = selector {
        let (layer, unit) = normalize(&selector).ok_or_else(|| err(pos, "E_IMPORT",
            format!("unknown compiler-owned {} module '{}'", reserved_layer_name(&selector), selector)))?;
        let canonical = format!("{}.{}", layer.name().trim_start_matches("atto."), unit);
        if selected.as_deref().is_some_and(|name| name != unit && name != canonical.as_str()) {
            return Err(err(pos, "E_IMPORT", "namespace selection does not match builtin identity"));
        }
        let v2 = !legacy_module;
        let mut aliases = Vec::new();
        if legacy_module {
            if requested.as_deref() != Some(unit) {
                return Err(err(pos, "E_IMPORT", "legacy module binding must match library unit"));
            }
            aliases.push(unit.to_string());
        } else if explicit_alias {
            aliases.push(requested.ok_or_else(|| err(pos, "E_IMPORT", "missing alias"))?);
        } else if requested.as_deref() == Some(canonical.as_str()) {
            aliases.push(canonical);
        } else if (direct && requested.is_none()) || requested.as_deref() == Some(unit) {
            aliases.push(unit.to_string());
            aliases.push(canonical);
        } else {
            return Err(err(pos, "E_IMPORT", "use a natural or canonical binding, or direct import with an explicit as alias"));
        }
        return Ok(Some(LibImport {
            binding: unit.to_string(),
            aliases,
            v2,
            selector,
            pos,
            start: 0,
            end: 0,
        }));
    }

    Ok(None)
}

/// Syntax tooling uses the same classifier as the executable builtin lane.
/// The return value includes only lexical bindings, never runtime objects.
pub(crate) fn builtin_bindings(tokens: &[Token], start: usize) -> R<Option<(Vec<String>, usize)>> {
    let mut end = start;
    while tokens.get(end).is_some_and(|t| !matches!(t.tok, Tok::Semi | Tok::Eof)) { end += 1; }
    let Some(body) = tokens.get(start..end) else { return Ok(None); };
    // Preserve legacy syntax-only external and typed symbol admission. An
    // explicit pkg domain is never a builtin, even for reserved spellings.
    // The executable dispatcher independently rejects unsupported resolution.
    if matches!(ident(body.get(1)), Some("pkg" | "const" | "fnc" | "class" | "actor" | "enum" | "type" | "trait" | "interface"))
        || body.iter().any(|t| ident(Some(t)) == Some("pkg"))
        || body.get(1).is_some_and(|t| t.tok == Tok::Star) {
        return Ok(None);
    }
    let Some(imp) = parse_builtin_clause(body)? else { return Ok(None); };
    if tokens.get(end).is_none_or(|t| t.tok != Tok::Semi) {
        return Err(err(imp.pos, "E_SYNTAX", "expected semicolon after builtin import"));
    }
    Ok(Some((imp.aliases, end + 1)))
}

/// Header-position library imports of the form
/// `import module <name> from "<selector>";` with a reserved selector.
fn library_imports(source: &str) -> R<Vec<LibImport>> {
    let toks = lexer::lex(source)?;
    let mut out = Vec::new();
    let mut i = 0;
    // Optional `use [default] namespace ...;` header.
    if ident(toks.get(i)) == Some("use") {
        while i < toks.len() && toks[i].tok != Tok::Semi {
            i += 1;
        }
        i += 1;
    }
    while ident(toks.get(i)) == Some("import") {
        let start_tok = i;
        let mut j = i;
        while j < toks.len() && !matches!(toks[j].tok, Tok::Semi | Tok::Eof) {
            j += 1;
        }
        if toks.get(j).map(|t| &t.tok) != Some(&Tok::Semi) {
            return Err(err(toks[start_tok].pos, "E_SYNTAX", "unterminated import"));
        }
        if let Some(mut imp) = parse_builtin_clause(&toks[start_tok..j])? {
            imp.start = byte_offset(source, imp.pos);
            imp.end = byte_offset(source, toks[j].pos) + 1;
            out.push(imp);
            if out.len() > crate::import_v2::MAX_ITEMS {
                return Err(err(toks[start_tok].pos, "E_SOURCE_LIMIT", "too many builtin import declarations"));
            }
        }
        i = j + 1;
    }
    if !out.is_empty() {
        // Reject late/duplicate file headers before erasing any imports. The
        // normalizer will still validate the actual header and logical path.
        for (i, window) in toks.windows(2).enumerate() {
            if i != 0 && ident(window.first()) == Some("use")
                && (ident(window.get(1)) == Some("namespace")
                    || (ident(window.get(1)) == Some("default")
                        && ident(toks.get(i + 2)) == Some("namespace"))) {
                let code = if ident(toks.first()) == Some("use") { "E_DUPLICATE" } else { "E_SYNTAX" };
                return Err(err(window[0].pos, code, "namespace header must precede imports and occur only once"));
            }
        }
    }
    Ok(out)
}

fn blank(source: &str, spans: &[(usize, usize)]) -> String {
    let mut bytes = source.as_bytes().to_vec();
    for &(start, end) in spans {
        for b in &mut bytes[start..end] {
            if *b != b'\n' {
                *b = b' ';
            }
        }
    }
    String::from_utf8(bytes).expect("blanking ASCII-delimited spans preserves UTF-8")
}

fn visit(
    importer: Option<Layer>,
    imp: &LibImport,
    os: TargetOs,
    stack: &mut Vec<(Layer, &'static str)>,
    done: &mut Vec<(Layer, &'static str, String)>,
) -> R<()> {
    let (layer, unit_name) = normalize(&imp.selector).ok_or_else(|| {
        err(imp.pos, "E_IMPORT", format!(
            "unknown compiler-owned {} module '{}'",
            reserved_layer_name(&imp.selector), imp.selector))
    })?;
    let unit = find(layer, unit_name, os).ok_or_else(|| {
        err(imp.pos, "E_IMPORT", format!("unknown compiler-owned {} module '{}'", layer.name(), imp.selector))
    })?;
    if let Some(from) = importer {
        if layer > from {
            return Err(err(imp.pos, "E_IMPORT", format!(
                "compiler-owned {} modules may not depend on {}/*", from.name(), layer.name())));
        }
    }
    if imp.binding != unit.name {
        return Err(err(imp.pos, "E_IMPORT", format!(
            "module binding '{}' must match library unit '{}'", imp.binding, unit.name)));
    }
    if done.iter().any(|(l, n, _)| *l == layer && *n == unit.name) {
        return Ok(());
    }
    if stack.contains(&(layer, unit.name)) {
        return Err(err(imp.pos, "E_IMPORT", "library import cycle"));
    }
    if done.len() >= MAX_UNITS {
        return Err(err(imp.pos, "E_SOURCE_LIMIT", "too many library units"));
    }
    stack.push((layer, unit.name));
    let deps = library_imports(unit.source)?;
    for dep in &deps {
        visit(Some(layer), dep, os, stack, done)?;
    }
    stack.pop();
    let spans: Vec<_> = deps.iter().map(|d| (d.start, d.end)).collect();
    done.push((layer, unit.name, blank(unit.source, &spans)));
    Ok(())
}

fn reserved_layer_name(selector: &str) -> &'static str {
    if selector.starts_with("alloc") || selector.starts_with("atto.alloc") {
        "atto.alloc"
    } else if selector.starts_with("std") || selector.starts_with("atto.std") || selector.starts_with("ores.std") {
        "atto.std"
    } else {
        "atto.core"
    }
}

/// Bounded source rewrite for direct calls only. All user spellings map
/// to the same existing checked module identity. Never expose the generated
/// natural module name when only a qualified or explicit alias was requested.
/// Reject shadowing, runtime namespace values, and mixed linker domains until
/// the full lexical export graph can represent them.
fn rewrite_v2(source: &str, imports: &[LibImport], units: &[(Layer, &'static str, String)]) -> R<String> {
    let mut aliases = BTreeMap::<Vec<String>, String>::new();
    let mut protected = BTreeSet::<String>::new();
    for (_, name, _) in units { protected.insert((*name).to_string()); }
    for imp in imports {
        for alias in &imp.aliases {
            let parts: Vec<_> = alias.split('.').map(str::to_string).collect();
            protected.insert(parts[0].clone());
            if aliases.keys().any(|prior| prior.starts_with(&parts) || parts.starts_with(prior)) {
                return Err(err(imp.pos, "E_DUPLICATE", "duplicate or overlapping builtin binding"));
            }
            aliases.insert(parts, imp.binding.clone());
        }
    }
    let tokens = lexer::lex(source)?;
    // Map line/column to byte offsets once, including Unicode source.
    let mut offsets = BTreeMap::new();
    let (mut line, mut col) = (1u32, 1u32);
    for (offset, ch) in source.char_indices() {
        offsets.insert((line, col), offset);
        if ch == '\n' { line += 1; col = 1; } else { col += 1; }
    }
    let mut i = 0usize;
    if ident(tokens.first()) == Some("use") {
        while i < tokens.len() && !matches!(tokens[i].tok, Tok::Semi | Tok::Eof) { i += 1; }
        if tokens.get(i).is_some_and(|t| t.tok == Tok::Semi) { i += 1; }
    }
    let mut edits = Vec::new();
    // This bounded lane admits only leading file-scope namespace aliases,
    // after imports and an optional validated namespace header. Ordinary
    // typed constants and other declarations stay in the production parser.
    // Resolve all aliases before erasing any of their declarations.
    let mut constants = Vec::<(String, String, Pos)>::new();
    while ident(tokens.get(i)) == Some("define")
        && ident(tokens.get(i + 1)) == Some("alias")
    {
        let pos = tokens[i].pos;
        let start = i + 2;
        let mut end = start;
        while tokens.get(end).is_some_and(|t| !matches!(t.tok, Tok::Semi | Tok::Eof)) { end += 1; }
        if tokens.get(end).is_none_or(|t| t.tok != Tok::Semi) {
            return Err(err(pos, "E_SYNTAX", "unterminated namespace alias"));
        }
        // Normalize both spellings into (new name, existing reference):
        // NAME = PATH, or PATH as NAME. Exactly one separator is allowed.
        let body = &tokens[start..end];
        let mut separators = body.iter().enumerate().filter(|(_, t)|
            t.tok == Tok::Assign || ident(Some(t)) == Some("as"));
        let (at, separator) = separators.next().ok_or_else(||
            err(pos, "E_SYNTAX", "expected define alias NAME = PATH or PATH as NAME"))?;
        if separators.next().is_some() {
            return Err(err(pos, "E_SYNTAX", "namespace alias requires exactly one separator"));
        }
        let (left, right) = (&body[..at], &body[at + 1..]);
        let (name_tokens, reference_tokens) = if separator.tok == Tok::Assign {
            (left, right)
        } else { (right, left) };
        if name_tokens.len() != 1 {
            return Err(err(pos, "E_SYNTAX", "namespace alias name must be a single identifier"));
        }
        let name = binding_path(name_tokens, pos)?;
        let reference = binding_path(reference_tokens, pos)?;
        if constants.len() >= crate::import_v2::MAX_ITEMS {
            return Err(err(pos, "E_SOURCE_LIMIT", "too many namespace aliases"));
        }
        // Conservative subset: do not shadow existing imported aliases or
        // hidden natural module names until full lexical binding is wired.
        if protected.contains(&name) || constants.iter().any(|(prior, _, _)| prior == &name) {
            return Err(err(pos, "E_DUPLICATE", "namespace alias collides with builtin binding"));
        }
        let start_byte = byte_offset(source, pos);
        let end_byte = byte_offset(source, tokens[end].pos) + 1;
        let span = source.get(start_byte..end_byte)
            .ok_or_else(|| err(pos, "E_INTERNAL", "invalid namespace alias source span"))?;
        let replacement: String = span.bytes().map(|b| if b == b'\n' { '\n' } else { ' ' }).collect();
        edits.push((start_byte, end_byte, replacement));
        constants.push((name, reference, pos));
        i = end + 1;
    }
    if !constants.is_empty() {
        use crate::static_import_graph::{Origin, Scope, Target, Tier};
        use crate::import_v2::Kind;
        let mut scope = Scope::new();
        for imp in imports {
            let (layer, name) = normalize(&imp.selector)
                .ok_or_else(|| err(imp.pos, "E_INTERNAL", "invalid admitted builtin identity"))?;
            let identity = format!("{}.{}", layer.name().trim_start_matches("atto."), name);
            for alias in &imp.aliases {
                scope.bind_verified(alias, Tier::Deliberate, Target {
                    origin: Origin::Builtin(identity.clone()),
                    declaration: name.to_string(),
                    kind: Kind::Namespace,
                })?;
            }
        }
        let declarations: Vec<_> = constants.iter().map(|(n,r,_)| (n.as_str(),r.as_str())).collect();
        scope.namespace_aliases(&declarations, &[])?;
        for (name, _, pos) in constants {
            let target = scope.resolve(&name)?.target;
            if !matches!(target.origin, Origin::Builtin(_)) {
                return Err(err(pos, "E_IMPORT_KIND", "namespace alias must resolve to admitted builtin"));
            }
            protected.insert(name.clone());
            aliases.insert(vec![name], target.declaration);
        }
    }
    while i < tokens.len() {
        let token = &tokens[i];
        if ident(Some(token)) == Some("import") {
            return Err(err(token.pos, "E_UNSUPPORTED", "mixed builtin and external/late imports require full lexical linker"));
        }
        let Some(name) = ident(Some(token)) else { i += 1; continue; };
        if !protected.contains(name) { i += 1; continue; }
        if i > 0 && tokens[i - 1].tok == Tok::Dot {
            return Err(err(token.pos, "E_UNSUPPORTED", "namespace binding cannot be a runtime object member"));
        }
        let mut matched = None;
        for (parts, unit) in &aliases {
            if parts[0] != name { continue; }
            let n = parts.len();
            if parts.iter().enumerate().all(|(j,p)| ident(tokens.get(i + j*2)) == Some(p.as_str())
                && (j == 0 || tokens.get(i + j*2 - 1).is_some_and(|t| t.tok == Tok::Dot)))
                && tokens.get(i + n*2 - 1).is_some_and(|t| t.tok == Tok::Dot)
                && ident(tokens.get(i + n*2)).is_some()
                && tokens.get(i + n*2 + 1).is_some_and(|t| t.tok == Tok::LParen)
            { matched = Some((n, unit)); break; }
        }
        let (n, unit) = matched.ok_or_else(|| err(token.pos, "E_UNSUPPORTED",
            "unbound, shadowed or non-call builtin namespace reference"))?;
        let last = &tokens[i + (n-1)*2];
        let start = *offsets.get(&(token.pos.line, token.pos.col))
            .ok_or_else(|| err(token.pos, "E_INTERNAL", "missing import alias source offset"))?;
        let end = offsets.get(&(last.pos.line, last.pos.col)).copied()
            .and_then(|offset| offset.checked_add(ident(Some(last))?.len()))
            .filter(|end| *end <= source.len() && *end >= start && source.is_char_boundary(*end))
            .ok_or_else(|| err(last.pos, "E_INTERNAL", "invalid import alias source span"))?;
        // Preserve original line count and, when the alias span is long
        // enough, byte columns. The replacement is placed at the first
        // token; comments/whitespace in the span become whitespace.
        let mut replacement = unit.clone();
        let span = source.get(start..end).ok_or_else(|| err(token.pos, "E_INTERNAL", "invalid alias UTF-8 span"))?;
        let mut skipped = 0usize;
        for ch in span.chars() {
            if ch == '\n' { replacement.push('\n'); }
            else if skipped < unit.len() { skipped += ch.len_utf8(); }
            else { for _ in 0..ch.len_utf8() { replacement.push(' '); } }
        }
        edits.push((start, end, replacement));
        i += n*2 + 1;
    }
    let mut result = source.to_string();
    for (start, end, replacement) in edits.into_iter().rev() {
        result.replace_range(start..end, &replacement);
    }
    Ok(result)
}

/// Expand library imports. Sources without them are returned unchanged.
pub fn expand(source: &str) -> R<String> { expand_inner(source, false) }
pub(crate) fn expand_for_file_linker(source: &str) -> R<String> { expand_inner(source, true) }
fn expand_inner(source: &str, allow_local: bool) -> R<String> {
    if !source.contains("import") { return Ok(source.to_string()); }
    crate::import_v2::preflight_with_local(source, allow_local)?;
    let imports = library_imports(source)?;
    if imports.is_empty() { return Ok(source.to_string()); }
    let os = target_os();
    let mut done = Vec::new();
    let mut seen = BTreeSet::new();
    for imp in &imports {
        if seen.len().saturating_add(imp.aliases.len()) > crate::import_v2::MAX_ITEMS {
            return Err(err(imp.pos, "E_SOURCE_LIMIT", "too many builtin import bindings"));
        }
        for alias in &imp.aliases {
            if !seen.insert(alias.clone()) {
                return Err(err(imp.pos, "E_DUPLICATE", format!("duplicate imported module binding '{alias}'")));
            }
        }
        visit(None, imp, os, &mut Vec::new(), &mut done)?;
    }
    let spans: Vec<_> = imports.iter().map(|i| (i.start, i.end)).collect();
    let mut out = blank(source, &spans);
    if imports.iter().any(|imp| imp.v2) { out = rewrite_v2(&out, &imports, &done)?; }
    for (_, _, text) in &done {
        out.push('\n');
        out.push_str(text);
    }
    Ok(out)
}

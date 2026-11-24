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
            return Ok(out);
        }
        let body = &toks[start_tok..j];
        // Inspect only the source after 'from', never an arbitrary string
        // elsewhere in the clause. Recognize malformed reserved bare paths
        // too, so they cannot fall through to a file loader.
        let source_at = body.iter().position(|t| matches!(&t.tok, Tok::Ident(s) if s == "from"))
            .map(|at| at + 1);
        let mut selector = None;
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
                    return Err(err(toks[start_tok].pos, "E_UNSUPPORTED",
                        "explicit package selector cannot use compiler-owned library resolution"));
                }
            } else if quoted.as_deref().is_some_and(is_reserved) {
                if source.len() != 1 {
                    return Err(err(toks[start_tok].pos, "E_IMPORT", "malformed builtin selector"));
                }
                selector = quoted;
            } else if bare_reserved {
                if source.len() != 3 || source[1].tok != Tok::Dot || ident(source.get(2)).is_none() {
                    return Err(err(toks[start_tok].pos, "E_IMPORT", "builtin selector must be a flat dotted identity"));
                }
                selector = Some(format!("{}.{}", ident(source.first()).unwrap(), ident(source.get(2)).unwrap()));
            }
        }
        if let Some(selector) = selector.filter(|s| is_reserved(s)) {
            let pos = toks[start_tok].pos;
            let source_shape = (j == start_tok + 5 && matches!(&toks[start_tok + 4].tok, Tok::Str(_)))
                || (j == start_tok + 7 && toks[start_tok + 5].tok == Tok::Dot
                    && ident(toks.get(start_tok + 4)).is_some()
                    && ident(toks.get(start_tok + 6)).is_some());
            let shape_ok = source_shape
                && matches!(ident(toks.get(start_tok + 1)), Some("module" | "namespace"))
                && ident(toks.get(start_tok + 3)) == Some("from");
            let binding = match (shape_ok, ident(toks.get(start_tok + 2))) {
                (true, Some(name)) if !crate::parser::is_reserved(name) => name.to_string(),
                _ => {
                    return Err(err(pos, "E_IMPORT",
                        "library units currently require natural-name module or namespace import; legacy 'import module <unit> from \"<layer>/<unit>\";' in this compiler"))
                }
            };
            out.push(LibImport {
                binding,
                selector,
                pos,
                start: byte_offset(source, pos),
                end: byte_offset(source, toks[j].pos) + 1,
            });
        }
        i = j + 1;
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

/// Expand library imports. Sources without them are returned unchanged.
pub fn expand(source: &str) -> R<String> {
    if !source.contains("import") {
        return Ok(source.to_string());
    }
    crate::import_v2::preflight(source)?;
    let imports = library_imports(source)?;
    if imports.is_empty() {
        return Ok(source.to_string());
    }
    let os = target_os();
    let mut done = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    for imp in &imports {
        if !seen.insert(imp.binding.clone()) {
            return Err(err(imp.pos, "E_DUPLICATE", format!("duplicate imported module binding '{}'", imp.binding)));
        }
        visit(None, imp, os, &mut Vec::new(), &mut done)?;
    }
    let spans: Vec<_> = imports.iter().map(|i| (i.start, i.end)).collect();
    let mut out = blank(source, &spans);
    for (_, _, text) in &done {
        out.push('\n');
        out.push_str(text);
    }
    Ok(out)
}

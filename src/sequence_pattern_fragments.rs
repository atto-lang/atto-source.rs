//! Statically typed std.pattern sequence fragments, frontend groundwork.
//!
//! This module is deliberately **not** part of executable Atto compilation.
//! The current compiler has no general List<T>/Array<T> guest ABI or checked
//! sequence pattern AST; calling this parser must never turn a template into
//! a runtime extractor or enable unverified Wasm emission.
//!
//! The caller must prove the canonical std.pattern symbol, pattern-position
//! provenance and literal/constexpr template identity before invoking it.
//! See atto-spec PR #59, language/typed-pattern-fragments-v1.md.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ElementType {
    Any,
    Int,
    Float,
    Bool,
    String,
}

impl ElementType {
    fn named(name: &str) -> Option<Self> {
        match name {
            "Any" => Some(Self::Any),
            "int" => Some(Self::Int),
            "float" => Some(Self::Float),
            "bool" => Some(Self::Bool),
            "string" => Some(Self::String),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Atom {
    One,
    Rest,
    Int(i64),
    Bool(bool),
    String(String),
    Is { ty: ElementType, negative: bool },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SeqPart {
    pub element_type: ElementType,
    pub atoms: Vec<Atom>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FragmentError {
    pub code: &'static str,
    pub message: &'static str,
}

type R<T> = Result<T, FragmentError>;

fn error(code: &'static str, message: &'static str) -> FragmentError {
    FragmentError { code, message }
}

/// Trusted compiler provenance; Runtime intentionally cannot be parsed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source<'a> {
    Literal(&'a str),
    ProvenPureConst(&'a str),
    Runtime,
}

const MAX_SOURCE_BYTES: usize = 4096;
const MAX_ITEMS: usize = 128;
const MAX_SEGMENTS: usize = 32;
const MAX_INPUT_ELEMENTS: usize = 256;
const MAX_DP_CELLS: usize = (MAX_ITEMS + 1) * (MAX_INPUT_ELEMENTS + 1);

/// Caller supplies a *resolved symbol identity* and contextual type.
/// No text-only name matching may authorize the intrinsic.
pub fn compile_seq_part(
    canonical_std_pattern_symbol: bool,
    in_sequence_pattern: bool,
    source: Source<'_>,
    contextual_type: ElementType,
    explicit_type: Option<ElementType>,
) -> R<SeqPart> {
    if !canonical_std_pattern_symbol || !in_sequence_pattern {
        return Err(error("E_PATTERN_CONTEXT", "requires canonical std.pattern intrinsic in sequence pattern"));
    }
    let text = match source {
        Source::Literal(text) | Source::ProvenPureConst(text) => text,
        Source::Runtime => return Err(error("E_PATTERN_CONSTEXPR", "pattern source must be proven at compile time")),
    };
    if explicit_type.is_some_and(|ty| ty != contextual_type) {
        return Err(error("E_PATTERN_TYPE", "explicit SeqPart element type must match contextual sequence type"));
    }
    parse_fragment(text, contextual_type)
}

/// Parse only the bounded binder-free *fragment* grammar, not Atto expressions.
/// Comma-separated items expand at the caller's pattern-item location.
fn parse_fragment(source: &str, element_type: ElementType) -> R<SeqPart> {
    if source.len() > MAX_SOURCE_BYTES {
        return Err(error("E_PATTERN_BUDGET", "fragment source exceeds size limit"));
    }
    if source.chars().any(|c| c == '\0' || matches!(c, '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')) {
        return Err(error("E_PATTERN_SOURCE", "unsafe Unicode control in pattern source"));
    }
    let parts = split_items(source)?;
    let mut atoms = Vec::new();
    for part in parts {
        if let Some(inner) = part.strip_prefix("...(").and_then(|s| s.strip_suffix(')')) {
            let segments = split_items(inner)?;
            if segments.len() > MAX_SEGMENTS {
                return Err(error("E_PATTERN_BUDGET", "too many counted segments"));
            }
            for segment in segments {
                let (n, predicate) = segment.split_once(" of ").ok_or_else(|| {
                    error("E_PATTERN_SOURCE", "expected N of pattern in counted segment")
                })?;
                if n.is_empty() || (n.len() > 1 && n.starts_with('0'))
                    || !n.bytes().all(|c| c.is_ascii_digit()) {
                    return Err(error("E_PATTERN_SOURCE", "count must be a canonical nonnegative literal"));
                }
                let count: usize = n.parse().map_err(|_| error(
                    "E_PATTERN_BUDGET", "count exceeds integer bounds"
                ))?;
                if count > MAX_ITEMS.saturating_sub(atoms.len()) {
                    return Err(error("E_PATTERN_BUDGET", "expanded fragment exceeds element budget"));
                }
                let p = if let Some(ty) = ElementType::named(predicate.trim()) {
                    if ty == ElementType::Any {
                        return Err(error("E_PATTERN_SOURCE", "Any requires an explicit type test"));
                    }
                    check_type_test(ty, false, element_type)?
                } else {
                    parse_atom(predicate.trim(), element_type)?
                };
                if matches!(p, Atom::Rest) {
                    return Err(error("E_PATTERN_SOURCE", "unbounded rests cannot occur in repetition"));
                }
                for _ in 0..count { atoms.push(p.clone()); }
            }
        } else {
            if atoms.len() >= MAX_ITEMS {
                return Err(error("E_PATTERN_BUDGET", "expanded fragment exceeds element budget"));
            }
            atoms.push(parse_atom(part, element_type)?);
        }
    }
    if atoms.windows(2).any(|pair| matches!(pair, [Atom::Rest, Atom::Rest])) {
        return Err(error("E_PATTERN_SOURCE", "adjacent rest wildcards are ambiguous"));
    }
    Ok(SeqPart { element_type, atoms })
}

/// Delimiter splitting is quote-aware and refuses unbalanced/unterminated text.
/// Counted groups may contain commas, but no nested groups or calls.
fn split_items(source: &str) -> R<Vec<&str>> {
    if source.trim().is_empty() {
        return Err(error("E_PATTERN_SOURCE", "empty fragment"));
    }
    let (mut start, mut depth, mut quoted, mut escaped) = (0usize, 0usize, false, false);
    let mut out = Vec::new();
    for (index, c) in source.char_indices() {
        if quoted {
            if escaped { escaped = false; }
            else if c == '\\' { escaped = true; }
            else if c == '"' { quoted = false; }
            continue;
        }
        match c {
            '"' => quoted = true,
            '(' => {
                depth += 1;
                if depth > 1 { return Err(error("E_PATTERN_SOURCE", "nested parentheses unsupported")); }
            },
            ')' => {
                if depth == 0 { return Err(error("E_PATTERN_SOURCE", "unbalanced parentheses")); }
                depth -= 1;
            },
            ',' if depth == 0 => {
                out.push(source[start..index].trim());
                start = index + 1;
            },
            _ => {}
        }
        if out.len() > MAX_ITEMS {
            return Err(error("E_PATTERN_BUDGET", "too many fragment items"));
        }
    }
    if quoted || escaped || depth != 0 {
        return Err(error("E_PATTERN_SOURCE", "unterminated string or group"));
    }
    out.push(source[start..].trim());
    if out.iter().any(|s| s.is_empty()) {
        return Err(error("E_PATTERN_SOURCE", "empty item or trailing comma"));
    }
    Ok(out)
}

fn check_type_test(
    ty: ElementType,
    negative: bool,
    expected: ElementType,
) -> R<Atom> {
    if ty == ElementType::Any {
        return Err(error("E_PATTERN_TYPE", "Any is not a reifiable discriminator"));
    }
    if expected != ElementType::Any &&
       ((!negative && expected != ty) || (negative && expected == ty)) {
        return Err(error("E_PATTERN_TYPE", "statically impossible element type test"));
    }
    Ok(Atom::Is { ty, negative })
}

fn parse_atom(source: &str, expected: ElementType) -> R<Atom> {
    if source.contains(" as ") {
        return Err(error("E_PATTERN_BINDING", "bindings belong outside pattern text"));
    }
    if source == "..." { return Ok(Atom::Rest); }
    if source == "_" { return Ok(Atom::One); }
    if let Some(type_name) = source.strip_prefix("is ") {
        let (negative, name) = if let Some(name) = type_name.strip_prefix('!') {
            (true, name)
        } else { (false, type_name) };
        let ty = ElementType::named(name.trim())
            .ok_or_else(|| error("E_PATTERN_TYPE", "unknown or non-reifiable type test"))?;
        return check_type_test(ty, negative, expected);
    }
    if source == "true" || source == "false" {
        if expected != ElementType::Any && expected != ElementType::Bool {
            return Err(error("E_PATTERN_TYPE", "boolean literal cannot match element type"));
        }
        return Ok(Atom::Bool(source == "true"));
    }
    if source.starts_with('"') && source.ends_with('"') && source.len() >= 2 {
        if expected != ElementType::Any && expected != ElementType::String {
            return Err(error("E_PATTERN_TYPE", "string literal cannot match element type"));
        }
        let inner = &source[1..source.len() - 1];
        let mut value = String::new();
        let mut chars = inner.chars();
        while let Some(c) = chars.next() {
            if c != '\\' {
                if c == '"' || c.is_control() { return Err(error("E_PATTERN_SOURCE", "invalid string literal")); }
                value.push(c);
            } else {
                let replacement = match chars.next() {
                    Some('"') => '"',
                    Some('\\') => '\\',
                    Some('n') => '\n',
                    Some('t') => '\t',
                    _ => return Err(error("E_PATTERN_SOURCE", "unsupported string escape")),
                };
                value.push(replacement);
            }
        }
        return Ok(Atom::String(value));
    }
    if let Ok(value) = source.parse::<i64>() {
        if expected != ElementType::Any && expected != ElementType::Int {
            return Err(error("E_PATTERN_TYPE", "integer literal cannot match element type"));
        }
        if source.starts_with('+') || source.starts_with("-0") || (source.starts_with('0') && source.len() > 1) {
            return Err(error("E_PATTERN_SOURCE", "noncanonical numeric literal"));
        }
        return Ok(Atom::Int(value));
    }
    Err(error("E_PATTERN_SOURCE", "unsupported fragment syntax or inline binding"))
}

/// Combine expanded fragments with ordinary checked sequence atoms.
/// This deliberately does NOT grant guest code generation.
pub fn compose(parts: &[SeqPart], ty: ElementType) -> R<SeqPart> {
    let mut atoms = Vec::new();
    for fragment in parts {
        if fragment.element_type != ty {
            return Err(error("E_PATTERN_TYPE", "fragment element types differ"));
        }
        if fragment.atoms.len() > MAX_ITEMS.saturating_sub(atoms.len()) {
            return Err(error("E_PATTERN_BUDGET", "combined pattern exceeds element budget"));
        }
        atoms.extend_from_slice(&fragment.atoms);
    }
    if atoms.windows(2).any(|pair| matches!(pair, [Atom::Rest, Atom::Rest])) {
        return Err(error("E_PATTERN_SOURCE", "adjacent rests after expansion"));
    }
    Ok(SeqPart { element_type: ty, atoms })
}

/// Reference-only bounded deterministic DP for scalar i64 slices.
/// Returns the [start,end) region consumed by each rest in source order.
/// Not a runtime List<T> guest matcher or an entry point to Wasm emission.
pub fn reference_match_i64(pattern: &SeqPart, values: &[i64]) -> R<Option<Vec<(usize, usize)>>> {
    if pattern.element_type != ElementType::Int {
        return Err(error("E_PATTERN_TYPE", "reference matcher requires int sequence"));
    }
    let (m, n) = (pattern.atoms.len(), values.len());
    if m > MAX_ITEMS || n > MAX_INPUT_ELEMENTS ||
       (m+1).checked_mul(n+1).is_none_or(|count| count > MAX_DP_CELLS) {
        return Err(error("E_PATTERN_BUDGET", "reference matcher state budget exceeded"));
    }
    let width = n + 1;
    let mut possible = vec![false; (m+1)*width];
    possible[m*width+n] = true;
    for i in (0..m).rev() {
        for j in (0..=n).rev() {
            possible[i*width+j] = match &pattern.atoms[i] {
                Atom::Rest => possible[(i+1)*width+j]
                    || (j<n && possible[i*width+j+1]),
                p => j<n && matches_i64(p,values[j]) && possible[(i+1)*width+j+1]
            };
        }
    }
    if !possible[0] { return Ok(None); }
    let (mut i,mut j) = (0usize,0usize);
    let mut regions = Vec::new();
    while i < m {
        if matches!(&pattern.atoms[i],Atom::Rest) {
            let start = j;
            while !possible[(i+1)*width+j] { j += 1; }
            regions.push((start,j));
        } else { j += 1; }
        i += 1;
    }
    Ok(Some(regions))
}

fn matches_i64(atom: &Atom, x: i64) -> bool {
    match atom {
        Atom::One => true,
        Atom::Int(v) => *v == x,
        Atom::Is {ty, negative} => (*ty == ElementType::Int) != *negative,
        Atom::Rest | Atom::Bool(_) | Atom::String(_) => false
    }
}

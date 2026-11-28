//! Language Server Protocol diagnostics v1 (no external dependencies).
//!
//! Turns compiler diagnostics into LSP `textDocument/publishDiagnostics`
//! notifications and handles base-protocol `Content-Length` framing, so an
//! editor extension (atto-vscode, atto-sublime, Neovim) can run the real
//! compiler as a language server without a JSON or LSP crate.
//!
//! Position correctness is the core of this slice. The compiler's `Pos` is
//! 1-based (line, Unicode-scalar column). LSP positions are 0-based
//! (line, **UTF-16 code unit**) by default. A naive `col - 1` misplaces
//! every squiggle after a non-BMP character (emoji, many CJK extensions),
//! so `to_lsp_position` re-measures the line prefix in UTF-16 and clamps
//! out-of-range positions to the line end instead of panicking.
use crate::diag::{Diagnostic, Pos};
use std::fmt::Write;

pub const MAX_FRAME_BYTES: usize = 4 * 1024 * 1024;
const MAX_HEADER_BYTES: usize = 1024;

/// 0-based LSP position (UTF-16 `character`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LspPosition {
    pub line: u32,
    pub character: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LspDiagnostic {
    pub start: LspPosition,
    pub end: LspPosition,
    /// 1 = Error (compiler diagnostics are fail-closed errors).
    pub severity: u8,
    pub code: String,
    pub message: String,
}

/// Convert a compiler `Pos` to an LSP UTF-16 position. Lines past EOF clamp
/// to the last line; columns past the line end clamp to the line end.
pub fn to_lsp_position(source: &str, pos: Pos) -> LspPosition {
    let lines: Vec<&str> = source.split('\n').collect();
    let line_index = (pos.line.max(1) as usize - 1).min(lines.len().saturating_sub(1));
    let line = lines.get(line_index).copied().unwrap_or("");
    let line = line.strip_suffix('\r').unwrap_or(line);
    let prefix_chars = pos.col.max(1) as usize - 1;
    let character: usize = line.chars().take(prefix_chars).map(char::len_utf16).sum();
    LspPosition {
        line: line_index as u32,
        character: character as u32,
    }
}

/// Range covering the character at `pos` (or an empty range at line end).
fn range_at(source: &str, pos: Pos) -> (LspPosition, LspPosition) {
    let start = to_lsp_position(source, pos);
    let next = to_lsp_position(
        source,
        Pos {
            line: pos.line,
            col: pos.col.saturating_add(1),
        },
    );
    (start, if next.line == start.line { next } else { start })
}

pub fn from_compiler(source: &str, d: &Diagnostic) -> LspDiagnostic {
    let (start, end) = range_at(source, d.pos);
    LspDiagnostic {
        start,
        end,
        severity: 1,
        code: d.code.to_string(),
        message: d.message.clone(),
    }
}

/// Run the real browser compiler and return LSP diagnostics. The compiler
/// is fail-fast, so a broken document yields exactly one diagnostic; a
/// document that compiles yields an empty list (which clears the editor).
pub fn diagnostics_for(source: &str) -> Vec<LspDiagnostic> {
    match crate::compile(source) {
        Ok(_) => Vec::new(),
        Err(d) => vec![from_compiler(source, &d)],
    }
}

/// RFC 8259 string escaping (control characters as \u00XX).
pub fn json_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 || c == '\u{7f}' || c == '\u{2028}' || c == '\u{2029}' => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn position_json(p: LspPosition) -> String {
    format!("{{\"line\":{},\"character\":{}}}", p.line, p.character)
}

/// `textDocument/publishDiagnostics` notification body.
pub fn publish_diagnostics_json(uri: &str, version: Option<i32>, diags: &[LspDiagnostic]) -> String {
    let items: Vec<String> = diags
        .iter()
        .map(|d| {
            format!(
                "{{\"range\":{{\"start\":{},\"end\":{}}},\"severity\":{},\"code\":{},\"source\":\"attoc\",\"message\":{}}}",
                position_json(d.start),
                position_json(d.end),
                d.severity,
                json_string(&d.code),
                json_string(&d.message)
            )
        })
        .collect();
    let version = version.map(|v| format!(",\"version\":{v}")).unwrap_or_default();
    format!(
        "{{\"jsonrpc\":\"2.0\",\"method\":\"textDocument/publishDiagnostics\",\"params\":{{\"uri\":{}{},\"diagnostics\":[{}]}}}}",
        json_string(uri),
        version,
        items.join(",")
    )
}

/// Base-protocol frame: `Content-Length: N\r\n\r\n<body>`.
pub fn frame(body: &str) -> Vec<u8> {
    let mut out = format!("Content-Length: {}\r\n\r\n", body.len()).into_bytes();
    out.extend_from_slice(body.as_bytes());
    out
}

#[derive(Debug, PartialEq, Eq)]
pub enum FrameError {
    HeaderTooLarge,
    MissingContentLength,
    DuplicateContentLength,
    InvalidContentLength,
    BodyTooLarge,
    InvalidUtf8,
}

/// Decode one frame from the front of `buf`. `Ok(None)` means more bytes
/// are needed. On success returns the UTF-8 body and the bytes consumed.
/// Unknown headers (e.g. Content-Type) are allowed per the spec; a missing,
/// duplicate, non-decimal or oversized Content-Length fails closed.
pub fn read_frame(buf: &[u8]) -> Result<Option<(String, usize)>, FrameError> {
    let header_end = match buf.windows(4).position(|w| w == b"\r\n\r\n") {
        Some(i) => i,
        None if buf.len() > MAX_HEADER_BYTES => return Err(FrameError::HeaderTooLarge),
        None => return Ok(None),
    };
    if header_end > MAX_HEADER_BYTES {
        return Err(FrameError::HeaderTooLarge);
    }
    let header = std::str::from_utf8(&buf[..header_end]).map_err(|_| FrameError::InvalidUtf8)?;
    let mut length: Option<usize> = None;
    for line in header.split("\r\n") {
        let Some((name, value)) = line.split_once(':') else { continue };
        if name.trim().eq_ignore_ascii_case("content-length") {
            let value = value.trim();
            if value.is_empty() || value.len() > 10 || !value.bytes().all(|b| b.is_ascii_digit()) {
                return Err(FrameError::InvalidContentLength);
            }
            if length
                .replace(value.parse().map_err(|_| FrameError::InvalidContentLength)?)
                .is_some()
            {
                return Err(FrameError::DuplicateContentLength);
            }
        }
    }
    let length = length.ok_or(FrameError::MissingContentLength)?;
    if length > MAX_FRAME_BYTES {
        return Err(FrameError::BodyTooLarge);
    }
    let start = header_end + 4;
    if buf.len() < start + length {
        return Ok(None);
    }
    let body = std::str::from_utf8(&buf[start..start + length]).map_err(|_| FrameError::InvalidUtf8)?;
    Ok(Some((body.to_string(), start + length)))
}

# Language-server diagnostics v1

Status: **draft, first executable slice** (Rust compiler library, no new
dependencies, wasm32-safe). Feature 3 of the 15 proposals.

## Goal

Editor feedback from the *real* compiler (never a second, divergent parser),
growing toward borrow visualization, inlay hints, quick fixes and refactors.

## Shipped (`src/lsp_diagnostics.rs`)

| API | Behaviour |
| --- | --- |
| `diagnostics_for(source)` | Runs `atto_compiler::compile`. Broken source gives exactly one error diagnostic (the compiler is fail-fast); valid source gives `[]`, which clears stale squiggles. |
| `to_lsp_position(source, Pos)` | Compiler `Pos` (1-based, Unicode-scalar column) to LSP (0-based, **UTF-16** column). Clamps out-of-range lines and columns; ignores CR in CRLF. |
| `publish_diagnostics_json(uri, version, diags)` | RFC 8259-escaped `textDocument/publishDiagnostics`, `source: "attoc"`, stable `code` (e.g. `E_UNBOUND`). |
| `frame` / `read_frame` | Base-protocol `Content-Length` framing. Partial input gives `Ok(None)`. Missing, duplicate, non-decimal or oversized lengths, a header over 1 KiB, a body over 4 MiB, or invalid UTF-8 fail closed. |

### Why UTF-16 matters

The compiler counts Unicode scalars. LSP counts UTF-16 code units. A naive
`col - 1` misplaces every squiggle after an emoji or other non-BMP character.
A regression test pins this: `"😀😀"; return nope;` puts `nope` at UTF-16
character 32, not 30.

## Tests

`tests/lsp_diagnostics.rs` (7 cases), all against the real compiler for
positions: ASCII mapping, the non-BMP shift, clearing on success, clamping and
CRLF, JSON escaping (quotes, control characters, U+2028), frame round trip
with partial and pipelined input, and malformed frames.

## Next slices

1. `attoc lsp` stdio loop: `initialize`, `didOpen`/`didChange` (full sync)
   to `publishDiagnostics`, with `positionEncoding` negotiation (UTF-8 when
   the client offers it). Reuse `IncrementalCompiler` (rs #521) and
   `ImportGraph::affected_by` to refresh only dependents.
2. **Multiple diagnostics:** collect recoverable checker errors instead of
   stopping at the first.
3. **Borrow visualization:** export loan and last-use ranges from the borrow
   checker (`borrow_export`) as LSP semantic tokens or decorations.
4. **Inlay hints:** inferred types and moved/borrowed markers at bindings.
5. **Quick fixes:** machine-applicable edits for stable codes
   (e.g. `'%' is reserved` gives `Integer.rem(a, b)`), then rename and
   extract-function refactors built on the typed AST.

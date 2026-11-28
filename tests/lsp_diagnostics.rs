//! LSP diagnostics v1: UTF-16 positions from the real compiler, JSON
//! notifications and fail-closed base-protocol framing.
use atto_compiler::lsp_diagnostics::{
    diagnostics_for, frame, json_string, publish_diagnostics_json, read_frame, to_lsp_position, FrameError, LspPosition,
};
use atto_compiler::Pos;

#[test]
fn ascii_error_maps_to_zero_based_position() {
    let src = "pub fnc main(): int { return missing; }";
    let d = diagnostics_for(src);
    assert_eq!(d.len(), 1);
    assert_eq!(d[0].code, "E_UNBOUND");
    assert_eq!(d[0].start, LspPosition { line: 0, character: 29 });
    assert_eq!(d[0].end, LspPosition { line: 0, character: 30 });
    assert_eq!(&src[29..36], "missing");
}

#[test]
fn non_bmp_characters_shift_utf16_columns() {
    // Two emoji are 2 Unicode scalars but 4 UTF-16 code units.
    let src = "pub fnc main(): int {\n  val string s = \"😀😀\"; return nope;\n}";
    let d = diagnostics_for(src);
    assert_eq!(d[0].code, "E_UNBOUND");
    assert_eq!(d[0].start, LspPosition { line: 1, character: 32 });
    let line: Vec<u16> = src.lines().nth(1).unwrap().encode_utf16().collect();
    assert_eq!(String::from_utf16(&line[32..36]).unwrap(), "nope");
}

#[test]
fn valid_document_clears_diagnostics() {
    assert!(diagnostics_for("pub fnc main(): int { return 42; }").is_empty());
}

#[test]
fn out_of_range_positions_clamp_and_crlf_is_ignored() {
    let src = "ab\r\ncd";
    assert_eq!(
        to_lsp_position(src, Pos { line: 1, col: 99 }),
        LspPosition { line: 0, character: 2 }
    );
    assert_eq!(to_lsp_position(src, Pos { line: 9, col: 1 }), LspPosition { line: 1, character: 0 });
    assert_eq!(to_lsp_position(src, Pos { line: 0, col: 0 }), LspPosition { line: 0, character: 0 });
}

#[test]
fn publish_notification_is_escaped_json() {
    let d = diagnostics_for("pub fnc main(): int { return missing; }");
    let json = publish_diagnostics_json("file:///w/a \"q\".atto", Some(3), &d);
    assert!(json.starts_with("{\"jsonrpc\":\"2.0\",\"method\":\"textDocument/publishDiagnostics\""));
    assert!(json.contains("\"uri\":\"file:///w/a \\\"q\\\".atto\",\"version\":3"));
    assert!(json.contains("\"range\":{\"start\":{\"line\":0,\"character\":29},\"end\":{\"line\":0,\"character\":30}}"));
    assert!(json.contains("\"severity\":1,\"code\":\"E_UNBOUND\",\"source\":\"attoc\""));
    assert_eq!(json_string("a\u{1}\n\u{2028}"), "\"a\\u0001\\n\\u2028\"");
    let cleared = publish_diagnostics_json("file:///x.atto", None, &[]);
    assert!(cleared.ends_with("\"diagnostics\":[]}}"));
}

#[test]
fn frame_round_trips_and_waits_for_partial_input() {
    let body = "{\"jsonrpc\":\"2.0\",\"method\":\"x\",\"params\":\"é\"}";
    let bytes = frame(body);
    assert_eq!(read_frame(&bytes[..10]), Ok(None));
    assert_eq!(read_frame(&bytes[..bytes.len() - 1]), Ok(None));
    let (decoded, used) = read_frame(&bytes).unwrap().unwrap();
    assert_eq!((decoded.as_str(), used), (body, bytes.len()));
    let mut two = bytes.clone();
    two.extend_from_slice(&frame("{}"));
    let (_, used) = read_frame(&two).unwrap().unwrap();
    assert_eq!(read_frame(&two[used..]).unwrap().unwrap().0, "{}");
}

#[test]
fn malformed_frames_fail_closed() {
    let with = |h: &str| format!("{h}\r\n\r\n{{}}").into_bytes();
    assert_eq!(read_frame(&with("Content-Type: x")), Err(FrameError::MissingContentLength));
    assert_eq!(
        read_frame(&with("Content-Length: 2\r\nContent-Length: 2")),
        Err(FrameError::DuplicateContentLength)
    );
    assert_eq!(read_frame(&with("Content-Length: -2")), Err(FrameError::InvalidContentLength));
    assert_eq!(
        read_frame(&with("Content-Length: 99999999999")),
        Err(FrameError::InvalidContentLength)
    );
    assert_eq!(read_frame(&with("Content-Length: 9999999")), Err(FrameError::BodyTooLarge));
    assert_eq!(read_frame(&vec![b'a'; 2000]), Err(FrameError::HeaderTooLarge));
    assert_eq!(read_frame(b"Content-Length: 1\r\n\r\n\xff"), Err(FrameError::InvalidUtf8));
    assert!(read_frame(&with("content-length: 2\r\nContent-Type: application/vscode-jsonrpc"))
        .unwrap()
        .is_some());
}

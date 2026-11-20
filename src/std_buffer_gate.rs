//! Versioned, fail-closed source-identity admission for canonical std.buffer.
//! This is not the std.buffer compiler implementation: until the class and
//! sealed OwnedBytes calls are typechecked/lowered, no guest bytes may emit.
//!
//! The browser compiler must NEVER infer trust from the package name alone.
//! A future loader must supply verified, immutable canonical bytes; these
//! hashes are pinned to atto-common-source #30 candidate canonical manifest.
//! Updating a hash requires a contract migration and conformance re-review.

use crate::diag::{Diagnostic, Pos};
use crate::lexer::{Tok, Token};
use std::collections::BTreeSet;

const CANDIDATE_SOURCES: [(&str, usize, &str); 4] = [
    ("stdlib/core/byte_bounds.atto", 726, "7465eb4b8e3325957d47fd0fc92206e75231c1ad140b5605cec56c5262cb5c7e"),
    ("stdlib/core/identity.atto", 55, "bed9da0b8cc8c9bee0e6e7fe302df1a1b2734699fd220f59af8a52ac67c18688"),
    ("stdlib/core/u32_endian.atto", 791, "18223c1ecfbe1f16b03540670d10ba13c6dc7fdc21be940f82fb621d8c943c67"),
    ("stdlib/std/buffer.atto", 4921, "ed7295c3c7a5e3278f089794881f59f0f09085412b8cf6faff40ee7a5b3714ba"),
];

fn diagnostic(pos: Pos, code: &'static str, message: &'static str) -> Diagnostic {
    Diagnostic::new(pos, code, message)
}

/// Authenticate an externally supplied library candidate *before* parsing it.
/// This is intentionally a distinct operation from compiling a guest. The
/// exact candidate files have not yet been admitted by the Rust typechecker.
pub fn verify_candidate_sources<'a>(
    sources: impl IntoIterator<Item = (&'a str, &'a str)>,
) -> Result<(), Diagnostic> {
    let mut present = BTreeSet::new();
    for (path, source) in sources {
        let Some((_, bytes, sha)) = CANDIDATE_SOURCES.iter().find(|(p, _, _)| *p == path) else {
            return Err(diagnostic(Pos::default(), "E_STDLIB_SOURCE",
                "unknown canonical stdlib source path"));
        };
        if !present.insert(path) {
            return Err(diagnostic(Pos::default(), "E_STDLIB_SOURCE",
                "duplicate canonical stdlib source path"));
        }
        if source.len() != *bytes ||
            crate::sha256::hex_digest(source.as_bytes()) != *sha {
            return Err(diagnostic(Pos::default(), "E_STDLIB_HASH",
                "canonical stdlib source hash/size differs from pinned manifest"));
        }
    }
    if present.len() != CANDIDATE_SOURCES.len() {
        return Err(diagnostic(Pos::default(), "E_STDLIB_SOURCE",
            "missing required pinned canonical stdlib source"));
    }
    Ok(())
}

/// Inspect actual lexer tokens, not substring matches in strings/comments.
/// Existing browsers cannot lower the source-level class or sealed OwnedBytes
/// ABI. A source import is a *dependency*, not permission to replace that
/// dependency with a same-name host module or guessed Wasm imports.
pub(crate) fn preflight(tokens: &[Token]) -> Result<(), Diagnostic> {
    for (at, token) in tokens.iter().enumerate() {
        if !matches!(&token.tok, Tok::Ident(name) if name == "import") {
            continue;
        }
        let word = |relative: usize, value: &str| {
            matches!(tokens.get(at + relative).map(|t| &t.tok),
                Some(Tok::Ident(name)) if name == value)
        };
        if !word(1, "module") || !word(3, "from") || !word(4, "pkg") {
            continue;
        }
        // Name-based package admission is not enough, even when the import
        // source is a correct, unmodified and signature-verified library.
        if matches!(tokens.get(at + 5).map(|t| &t.tok),
            Some(Tok::Str(name)) if name == "std.buffer") {
            if !matches!(tokens.get(at + 2).map(|t| &t.tok), Some(Tok::Ident(_))) ||
               tokens.get(at + 6).map(|t| &t.tok) != Some(&Tok::Semi) {
                return Err(diagnostic(token.pos, "E_STDLIB_IMPORT",
                    "invalid canonical std.buffer package import syntax"));
            }
            return Err(diagnostic(token.pos, "E_STDLIB_UNLINKED",
                "canonical std.buffer source and sealed OwnedBytes are not yet linked by the ordinary Rust/Wasm compiler"));
        }
        return Err(diagnostic(token.pos, "E_UNSUPPORTED",
            "package import has no authenticated linked dependency in browser compiler"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const EXAMPLE: &str = r#"import module buffer from pkg "std.buffer";
pub fnc main(): int throws Error {
  val mut bytes = buffer.ByteBuffer.allocate(8);
  bytes.write_u32_le(0, 0x12345678);
  return bytes.read_u32_le(0) eq 0x12345678 ? 1 : 0;
}
"#;

    #[test]
    fn canonical_example_is_a_precise_unlinked_gate_not_forged_guest_execution() {
        assert_eq!(crate::compile(EXAMPLE).unwrap_err().code, "E_STDLIB_UNLINKED");
        assert_eq!(crate::compile_with_imports(EXAMPLE,
            std::path::Path::new("src/main.atto"), |_| panic!("no file reads allowed"))
            .unwrap_err().code, "E_STDLIB_UNLINKED");
    }

    #[test]
    fn lexed_import_cannot_be_spoofed_by_comments_or_strings() {
        let good = r#"pub fnc main(): int {
  // import module buffer from pkg "std.buffer";
  return 1;
}"#;
        assert!(preflight(&crate::lexer::lex(good).unwrap()).is_ok());
        let fake = r#"import module buffer from pkg "std.unknown";
pub fnc main(): int { return 1; }"#;
        assert_eq!(crate::compile(fake).unwrap_err().code, "E_UNSUPPORTED");
        let bad = EXAMPLE.replace("from pkg \"std.buffer\";",
            "from pkg \"std.buffer\" as stolen;");
        assert_eq!(crate::compile(&bad).unwrap_err().code, "E_STDLIB_IMPORT");
        let bad = EXAMPLE.replace("from pkg \"std.buffer\";",
            "from pkg \"std.buffer\""); // missing semicolon
        assert_eq!(crate::compile(&bad).unwrap_err().code, "E_STDLIB_IMPORT");
    }

    #[test]
    fn library_bundle_admission_rejects_missing_unknown_duplicate_and_tamper() {
        let none: Vec<(&str, &str)> = Vec::new();
        assert_eq!(verify_candidate_sources(none).unwrap_err().code, "E_STDLIB_SOURCE");
        assert_eq!(verify_candidate_sources([("bad/path.atto", "source")])
            .unwrap_err().code, "E_STDLIB_SOURCE");
        assert_eq!(verify_candidate_sources([
            ("stdlib/core/byte_bounds.atto", "tampered"),
        ]).unwrap_err().code, "E_STDLIB_HASH");
        assert_eq!(verify_candidate_sources([
            ("stdlib/core/byte_bounds.atto", "tampered"),
            ("stdlib/core/byte_bounds.atto", "tampered"),
        ]).unwrap_err().code, "E_STDLIB_HASH");
    }

    // These are read-only, byte-identical test snapshots of the four canonical
    // candidate sources. The SHA256 manifest is authoritative; no test fixture
    // must ever become an implicit stdlib host fallback.
    const CANONICAL_BOUNDS: &str = include_str!("../tests/fixtures/canonical-buffer-v1/stdlib/core/byte_bounds.atto");
    const CANONICAL_IDENTITY: &str = include_str!("../tests/fixtures/canonical-buffer-v1/stdlib/core/identity.atto");
    const CANONICAL_ENDIAN: &str = include_str!("../tests/fixtures/canonical-buffer-v1/stdlib/core/u32_endian.atto");
    const CANONICAL_BUFFER: &str = include_str!("../tests/fixtures/canonical-buffer-v1/stdlib/std/buffer.atto");

    fn authentic_sources<'a>() -> [(&'a str, &'a str); 4] {
        [
            ("stdlib/core/byte_bounds.atto", CANONICAL_BOUNDS),
            ("stdlib/core/identity.atto", CANONICAL_IDENTITY),
            ("stdlib/core/u32_endian.atto", CANONICAL_ENDIAN),
            ("stdlib/std/buffer.atto", CANONICAL_BUFFER),
        ]
    }

    #[test]
    fn original_canonical_source_bytes_are_accepted_only_for_identity_admission() {
        assert!(verify_candidate_sources(authentic_sources()).is_ok());
        // Even after verifying all four sources, the compiler may not emit
        // a guest until it actually parses and links the canonical class.
        assert_eq!(crate::compile(EXAMPLE).unwrap_err().code, "E_STDLIB_UNLINKED");
    }

    #[test]
    fn a_changed_or_reordered_bundle_cannot_override_a_stdlib_source() {
        let mut sources = authentic_sources();
        let tampered = CANONICAL_BUFFER.replace("write_u32_le", "write_u32_be");
        sources[3].1 = &tampered;
        assert_eq!(verify_candidate_sources(sources).unwrap_err().code, "E_STDLIB_HASH");
        let missing = authentic_sources();
        assert_eq!(verify_candidate_sources(missing[..3].iter().copied())
            .unwrap_err().code, "E_STDLIB_SOURCE");
        let mut duplicate = authentic_sources();
        duplicate[3] = duplicate[0];
        assert_eq!(verify_candidate_sources(duplicate).unwrap_err().code, "E_STDLIB_SOURCE");
        let mut reordered = authentic_sources();
        reordered.reverse();
        assert!(verify_candidate_sources(reordered).is_ok());
    }

    #[test]
    fn source_manifest_is_complete_exactly_four_unique_byte_pins() {
        assert_eq!(CANDIDATE_SOURCES.len(), 4);
        let mut unique = BTreeSet::new();
        for (path, bytes, hash) in CANDIDATE_SOURCES {
            assert!(unique.insert(path));
            assert!(path.ends_with(".atto"));
            assert!(bytes > 0);
            assert_eq!(hash.len(), 64);
            assert!(hash.bytes().all(|b| b.is_ascii_hexdigit()));
        }
    }
}

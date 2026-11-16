//! Native atto_compiler coverage for the exact 42 published /play preview samples.
//! This is a smoke test, NOT evidence that each preview is executable.
use std::fs;
use std::path::Path;

#[test]
fn all_published_previews_compile_or_reject_with_a_diagnostic() {
    let dir = Path::new("tests/playground_previews");
    let manifest = fs::read_to_string(dir.join("MANIFEST.tsv")).expect("preview manifest");
    let entries: Vec<_> = manifest.lines()
        .filter(|s| !s.starts_with('#') && !s.is_empty()).collect();
    assert_eq!(entries.len(), 42, "all 42 public previews must be represented");
    let files: Vec<_> = fs::read_dir(dir).unwrap().map(|e| {
        e.unwrap().file_name().to_string_lossy().into_owned()
    }).filter(|s| s.ends_with(".atto")).collect();
    assert_eq!(files.len(), entries.len(), "no missing or untracked .atto files");
    let mut seen = std::collections::BTreeSet::new();
    for entry in entries {
        let (name, sha) = entry.split_once('\t').expect("name and Git blob SHA-1");
        assert_eq!(sha.len(), 40, "{name}: source SHA is missing");
        assert!(seen.insert(name.to_string()), "{name}: duplicate preview");
        assert!(files.contains(&format!("{name}.atto")), "{name}: missing file");
        let source = fs::read_to_string(dir.join(format!("{name}.atto"))).unwrap();
        let built = std::panic::catch_unwind(|| atto_compiler::compile(&source))
            .unwrap_or_else(|_| panic!("{name}: compiler panicked instead of diagnosing"));
        match built {
            Ok(bytes) => {
                assert!(bytes.starts_with(b"\0asm\x01\0\0\0"), "{name}: invalid Wasm header");
                assert_eq!(atto_compiler::compile(&source).unwrap(), bytes,
                    "{name}: nondeterministic output");
                eprintln!("PLAY_PREVIEW_ADMITTED_NOT_YET_VERIFIED {name}");
            }
            Err(diagnostic) => {
                assert!(diagnostic.code.starts_with("E_"),
                    "{name}: missing structured error: {diagnostic}");
                assert!(diagnostic.pos.line > 0 && diagnostic.pos.col > 0,
                    "{name}: missing positioned error");
                eprintln!("PLAY_PREVIEW_UNSUPPORTED {name}: {}", diagnostic.code);
            }
        }
    }
}

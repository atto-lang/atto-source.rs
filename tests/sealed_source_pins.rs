//! Source catalogue digests bind exact bytes; version pinning cannot
//! manufacture publisher authenticity or silently admit a second package ABI.
use atto_compiler::{
    source_digest::{sha256_hex,valid_digest},
    source_link::{compile_pinned,compile_linked,PinnedSourceUnit,SourceUnit},
};
const BASE:&str=r#"import class Dog from "animals/pets/dog";
export pub class Animal;
define sealed class Animal permits Dog as pub answer(): int {return 42;} end"#;
const DOG:&str=r#"import class Animal from "animals/core/base";
export pub class Dog;
define final class Dog extends Animal as end"#;
const ROOT:&str=r#"import class Dog from "animals/pets/dog";
pub fnc main(): int {val instance = new Dog(); return instance.answer();}"#;
fn units<'a>(base:&'a str,dog:&'a str,root:&'a str)->Vec<SourceUnit<'a>> {
    vec![
      SourceUnit{package:"animals",namespace:"core",module:"base",source:base},
      SourceUnit{package:"animals",namespace:"pets",module:"dog",source:dog},
      SourceUnit{package:"app",namespace:"main",module:"root",source:root}
    ]
}
fn pinned<'a>(plain:Vec<SourceUnit<'a>>,digests:&'a [String;3],versions:&'a [&'a str;3])->Vec<PinnedSourceUnit<'a>> {
    plain.into_iter().enumerate().map(|(i,unit)|PinnedSourceUnit {
        unit,package_version:versions[i],source_sha256:&digests[i]
    }).collect()
}
fn hashes()->[String;3] {
    [sha256_hex(BASE.as_bytes()),sha256_hex(DOG.as_bytes()),sha256_hex(ROOT.as_bytes())]
}
fn code(plain:&[PinnedSourceUnit<'_>],expected:&str) {
    let diagnostic=compile_pinned(plain,2).expect_err("unverified source bytes generated a guest");
    assert_eq!(diagnostic.code,expected,"{diagnostic}");
}
#[test]
fn sha256_matches_independent_standard_vectors_and_exact_utf8_bytes() {
    for (bytes,expected) in [
        ("".as_bytes(),"e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"),
        ("abc".as_bytes(),"ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"),
        ("Atto\0source".as_bytes(),"ad6f06303d5e930ddeb62973d9cddf978e560464d3fbda4a23bd2ee10c3e6497")
    ] {
        assert_eq!(sha256_hex(bytes),expected);
        assert!(valid_digest(expected));
        assert!(!valid_digest(&expected.to_uppercase()));
    }
    assert_eq!(sha256_hex(&vec![b'a';55]),"9f4390f8d30c2dd92ec9f095b65e2b9ae9b0a925a5258e241c9f1e910f734318");
    assert_eq!(sha256_hex(&vec![b'a';56]),"b35439a4ac6f0948b6d6f9e3c6af0f5f590ce20f1bde7090ef7970686ec6738a");
    assert_eq!(sha256_hex(&vec![b'a';63]),"7d3e74a05d7db15bce4ad9ec0658ea98e3f06eeecf16b4c6fff2da457ddc2f34");
    assert_eq!(sha256_hex(&vec![b'a';64]),"ffe054fe7ae0cb6dc65c3af9b61d5209f439851db43d0ba5997337df154668eb");
}
#[test]
fn pinned_graph_compiles_and_keeps_deterministic_source_receipt() {
    let digests=hashes();
    let versions=["1.2.0","1.2.0","2.0.0"];
    let result=compile_pinned(&pinned(units(BASE,DOG,ROOT),&digests,&versions),2).unwrap();
    assert!(result.starts_with(b"\0asm\x01\0\0\0"));
    assert_eq!(result,compile_pinned(&pinned(units(BASE,DOG,ROOT),&digests,&versions),2).unwrap());
    assert!(result.windows("atto.source.catalogue.v1".len())
        .any(|w|w==b"atto.source.catalogue.v1"));
    for digest in &digests {
        assert!(result.windows(digest.len()).any(|w|w==digest.as_bytes()),
            "source digest must appear in closed-program receipt");
    }
    let unpinned=compile_linked(&units(BASE,DOG,ROOT),2).unwrap();
    assert!(!unpinned.windows("atto.source.catalogue.v1".len())
        .any(|w|w==b"atto.source.catalogue.v1"),
        "legacy local-only path must not pretend to have a source receipt");
}
#[test]
fn changed_source_bytes_or_bad_digest_format_fail_before_compiling() {
    let mut digests=hashes();
    let versions=["1.2.0","1.2.0","2.0.0"];
    let modified=format!("{DOG}\n// changed after package lock");
    code(&pinned(units(BASE,&modified,ROOT),&digests,&versions),"E_SOURCE_DIGEST");
    digests[1]="A".repeat(64);
    code(&pinned(units(BASE,DOG,ROOT),&digests,&versions),"E_SOURCE_DIGEST");
    digests[1]="a".repeat(63);
    code(&pinned(units(BASE,DOG,ROOT),&digests,&versions),"E_SOURCE_DIGEST");
}
#[test]
fn package_versions_are_unique_across_modules_in_closed_link() {
    let digests=hashes();
    let versions=["1.2.0","9.9.9","2.0.0"];
    code(&pinned(units(BASE,DOG,ROOT),&digests,&versions),"E_PACKAGE_VERSION");
    let versions=["1.2.0","1.2.0","../evil"];
    code(&pinned(units(BASE,DOG,ROOT),&digests,&versions),"E_NAMESPACE");
}
#[test]
fn ambiguous_identifiers_are_not_rewritten_as_class_references() {
    for root in [
        "import class Dog from \"animals/pets/dog\"; pub fnc main(): int { val Dog = 3; return Dog; }",
        "import class Dog from \"animals/pets/dog\"; pub fnc Dog(): int {return 42;} pub fnc main(): int {return Dog();}",
        "import class Dog from \"animals/pets/dog\"; pub fnc main(): int {return Dog;}"
    ] {
        let diagnostic=compile_linked(&units(BASE,DOG,root),2).expect_err(
            "ambiguous bare imported class identifier was rewritten rather than rejected");
        assert_eq!(diagnostic.code,"E_NAMESPACE","{diagnostic}");
    }
}
#[test]
fn comments_and_strings_do_not_forge_import_class_usage() {
    let source=r#"import class Dog from "animals/pets/dog";
// class Dog is a comment, not an identity statement.
pub fnc main(): int {val label = "Dog"; val item = new Dog(); return item.answer();}"#;
    let digests=[sha256_hex(BASE.as_bytes()),sha256_hex(DOG.as_bytes()),sha256_hex(source.as_bytes())];
    let versions=["1.2.0","1.2.0","2.0.0"];
    assert!(compile_pinned(&pinned(units(BASE,DOG,source),&digests,&versions),2).is_ok());
}

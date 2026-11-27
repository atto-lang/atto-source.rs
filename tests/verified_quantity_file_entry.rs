use std::path::Path;

const PRE: &str = "define dimension Time; define unit Second: Time; type Positive = int where self > 0 and units in Second; ";

#[test]
fn typed_file_entry_and_direct_compiler_agree_without_import_loader() {
    let source = format!("{PRE}pub fnc main() -> int {{ val a=42<Second>; return a/1<Second>; }}");
    let direct = atto_compiler::compile(&source).expect("direct quantity lane");
    let linked = atto_compiler::compile_with_imports(&source, Path::new("main.atto"), |_| {
        panic!("file-local quantity lane must not consult loader")
    }).expect("file entry");
    assert_eq!(direct, linked);
}

#[test]
fn typed_file_entry_rejects_unsupported_import_and_unit_erasure() {
    for body in [
        "import fnc other from \"other.atto\"; pub fnc main() -> int { return 42; }",
        "pub fnc main() -> int { val a=42<Second>; return a; }",
    ] {
        let source = format!("{PRE}{body}");
        assert!(atto_compiler::compile_with_imports(&source, Path::new("main.atto"), |_| {
            panic!("unsupported quantity import must not consult loader")
        }).is_err());
    }
}

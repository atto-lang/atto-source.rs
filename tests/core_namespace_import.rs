//! Namespace-only Atto core source linking (no ambient module/registry lookup).
use atto_compiler::linker::{compile_linked, LinkUnit};

const MAIN: &str = r#"use namespace app.entry;
import namespace slice from "+core/slice.atto";
import namespace iter from "+core/iter.atto";
import namespace cmp from "+core/cmp.atto";
pub fnc main(): int {
  if !slice.valid_range(1, 2, 3) then return 1; fi
  if iter.next_index(0, 3, 1, 3).unwrap_or(-1) != 1 then return 2; fi
  if cmp.compare_int(40, 39) != 1 then return 3; fi
  return 42;
}"#;
const SLICE: &str = include_str!("../core/slice.atto");
const ITER: &str = include_str!("../core/iter.atto");
const CMP: &str = include_str!("../core/cmp.atto");
fn graph<'a>(main: &'a str, slice: &'a str, iter: &'a str, cmp: &'a str) -> [LinkUnit<'a>; 4] {
    [
        LinkUnit {path:"src/main.atto",package:"app@1.0.0",source:main},
        LinkUnit {path:"core/slice.atto",package:"app@1.0.0",source:slice},
        LinkUnit {path:"core/iter.atto",package:"app@1.0.0",source:iter},
        LinkUnit {path:"core/cmp.atto",package:"app@1.0.0",source:cmp},
    ]
}
fn reject(main: &str, slice: &str, iter: &str, cmp: &str, code: &str) {
    let failed = compile_linked("src/main.atto", &graph(main,slice,iter,cmp))
        .expect_err("untrusted namespace import must not produce Wasm");
    assert_eq!(failed.code,code,"unexpected diagnostic: {failed}");
}
#[test]
fn namespace_scoped_library_import_compiles_guest_deterministically() {
    let units = graph(MAIN,SLICE,ITER,CMP);
    let a = compile_linked("src/main.atto",&units).expect("namespace linked Wasm");
    assert!(a.starts_with(b"\0asm\x01\0\0\0"));
    assert_eq!(a,compile_linked("src/main.atto",&units).unwrap());
}
#[test]
fn namespace_identity_is_source_owned_and_must_match_alias() {
    let spoofed=SLICE.replace("use namespace atto.core.slice;", "use namespace atto.std.slice;");
    // No lexical namespace is trusted as an implicit virtual core source:
    // path and source declaration must agree under the explicit graph.
    reject(MAIN,&spoofed,ITER,CMP,"E_NAMESPACE_PATH");
    let mismatch=SLICE.replace("use namespace atto.core.slice;", "use namespace atto.core.not_slice;");
    reject(MAIN,&mismatch,ITER,CMP,"E_UNBOUND");
    reject(MAIN,&SLICE.replacen("use namespace atto.core.slice;","",1),ITER,CMP,"E_UNBOUND");
}
#[test]
fn undeclared_module_import_cannot_bind_to_namespace_source() {
    let legacy=MAIN.replace("import namespace slice","import module slice");
    reject(&legacy,SLICE,ITER,CMP,"E_UNBOUND");
}
#[test]
fn a_namespace_without_public_functions_is_not_importable() {
    let no_public=SLICE.replace("pub fnc ", "fnc ");
    reject(MAIN,&no_public,ITER,CMP,"E_VISIBILITY");
}
#[test]
fn namespace_reference_requires_local_explicit_import() {
    let main=MAIN.replace("import namespace cmp from \"+core/cmp.atto\";","")
        .replace("import namespace iter from \"+core/iter.atto\";","");
    let a=compile_linked("src/main.atto",&[
        LinkUnit{path:"src/main.atto",package:"app",source:&main},
        LinkUnit{path:"core/slice.atto",package:"app",source:SLICE}
    ]);
    assert!(a.is_err(), "missing namespace dependencies cannot be linked");
}

#[test]
fn reserved_namespace_names_are_not_forgeable_from_unrelated_sources() {
    let main = MAIN.replace("+core/slice.atto", "+vendor/slice.atto");
    let mut units = graph(&main, SLICE, ITER, CMP);
    units[1].path = "vendor/slice.atto";
    let failure = compile_linked("src/main.atto", &units)
        .expect_err("vendor source must not masquerade as core namespace");
    assert_eq!(failure.code, "E_NAMESPACE_PATH");
}

#[test]
fn no_namespace_import_is_inferred_from_an_entry_namespace_header() {
    let consumer = MAIN.replace("import namespace slice from \"+core/slice.atto\";", "");
    let failure = compile_linked("src/main.atto", &graph(&consumer, SLICE, ITER, CMP))
        .expect_err("reachable graph must contain explicit imports");
    assert_eq!(failure.code, "E_UNBOUND");
}

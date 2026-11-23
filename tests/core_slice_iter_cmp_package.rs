//! Compile real source modules through the deterministic production linker.
//! A Wasm magic header is compiler output, NOT proof of runtime guest execution.
use atto_compiler::linker::{compile_linked, LinkUnit};

const ENTRY: &str = include_str!("../core/tests/slice_iter_cmp_smoke.atto");
const SLICE: &str = include_str!("../core/slice.atto");
const ITER: &str = include_str!("../core/iter.atto");
const CMP: &str = include_str!("../core/cmp.atto");

fn units<'a>(entry: &'a str, slice: &'a str, iter: &'a str, cmp: &'a str)
    -> [LinkUnit<'a>; 4]
{
    [
        LinkUnit { path: "entry.atto", package: "app@1.0.0", source: entry },
        LinkUnit { path: "core/slice.atto", package: "app@1.0.0", source: slice },
        LinkUnit { path: "core/iter.atto", package: "app@1.0.0", source: iter },
        LinkUnit { path: "core/cmp.atto", package: "app@1.0.0", source: cmp },
    ]
}

#[test]
fn actual_core_scalar_sources_link_deterministically_to_wasm() {
    let graph = units(ENTRY, SLICE, ITER, CMP);
    let wasm = compile_linked("entry.atto", &graph).expect("compiled linked scalar core guest");
    assert!(wasm.starts_with(b"\0asm\x01\0\0\0"));
    assert_eq!(wasm, compile_linked("entry.atto", &graph).unwrap());
    let reversed = [graph[3], graph[2], graph[1], graph[0]];
    assert_eq!(wasm, compile_linked("entry.atto", &reversed).unwrap());
}

#[test]
fn bad_core_imports_never_emit_wasm() {
    let mutated = ENTRY.replace("+core/cmp.atto", "+../core/cmp.atto");
    let failure = compile_linked("entry.atto", &units(&mutated, SLICE, ITER, CMP))
        .expect_err("traversal must fail closed");
    assert_eq!(failure.code, "E_IMPORT_PATH");
    let missing = ENTRY.replace("+core/cmp.atto", "+core/missing.atto");
    let failure = compile_linked("entry.atto", &units(&missing, SLICE, ITER, CMP))
        .expect_err("missing core path must fail closed");
    assert_eq!(failure.code, "E_UNBOUND");
}

#[test]
fn scalar_module_must_not_masquerade_as_generic_array_support() {
    // No Array<T>, Fnc<T>, or fabricated borrowed Slice<T> is admitted here.
    for source in [SLICE, ITER, CMP] {
        assert!(!source.contains("Array<"));
        assert!(!source.contains("Fnc<"));
        assert!(!source.contains("define class Slice"));
    }
}

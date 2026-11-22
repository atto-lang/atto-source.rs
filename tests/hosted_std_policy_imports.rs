//! Compiled Wasm via the production compiler, not a regex or source hash pass.
//! Execution (main() == 42) is in hosted_io_pure_policy_guest.mjs.
//! These sealed *pure-policy* std packages do not implement network or file I/O.
use atto_compiler::{compile, compile_with_imports, Diagnostic};
use std::path::Path;

const POSITIVE: &str = r#"
import * as http from "std.http";
import * as fs from "std.fs";
pub fnc main(): int {
    if http.http_status_success(200) == false then return 1; fi
    if http.http_status_client_error(404) == false then return 2; fi
    if http.http_get_status_may_have_body(204) then return 3; fi
    if fs.fs_text_range_admissible(9, 2, 10) then return 4; fi
    if fs.fs_text_replacement_admissible(4, 7, 5, 8) == false then return 5; fi
    return 42;
}
"#;

fn code(source: &str) -> &'static str {
    let err = compile(source).expect_err("untrusted/unsupported import must fail without Wasm");
    err.code
}

#[test]
fn sealed_std_package_imports_reuse_real_wasm_compiler_without_host_io_imports() {
    let wasm = compile(POSITIVE).expect("sealed pure-policy package imports must compile");
    assert!(wasm.starts_with(b"\0asm\x01\0\0\0"), "real guest Wasm");
    assert_eq!(wasm, compile(POSITIVE).unwrap(), "deterministic bytes");
    assert!(!wasm.windows(b"atto_fs_v1".len()).any(|w| w == b"atto_fs_v1"));
    assert!(!wasm.windows(b"atto_http_v1".len()).any(|w| w == b"atto_http_v1"));
}

#[test]
fn native_compiler_cannot_inject_untrusted_files_over_sealed_std_sources() {
    let mut requests = 0;
    let wasm = compile_with_imports(POSITIVE, Path::new("project/main.atto"), |_path| {
        requests += 1;
        Err(std::io::Error::new(std::io::ErrorKind::PermissionDenied,
            "sealed stdlib must never consult the untrusted source loader"))
    }).unwrap();
    assert_eq!(requests, 0);
    assert_eq!(wasm, compile(POSITIVE).unwrap());
}

#[test]
fn selective_std_exports_and_aliases_link_with_real_wasm_emission() {
    let source = r#"
import http_status_success as ok from pkg "std.http";
import fs_text_size_admissible as fits from pkg "std.fs";
pub fnc main(): int {
    if ok(200) == false then return 1; fi
    if fits(10,10,10) == false then return 2; fi
    return 42;
}"#;
    assert!(compile(source).unwrap().starts_with(b"\0asm"));
}

#[test]
fn unknown_packages_runtime_io_and_forged_names_fail_closed() {
    for (source, expected) in [
        (r#"import * as x from pkg "core.telemetry";
pub fnc main(): int { return 0; }"#, "E_UNSUPPORTED"),
        (r#"import * as http from pkg "std.http";
pub fnc main(): int { return http.get(1); }"#, "E_UNBOUND"),
        (r#"import * as fs from pkg "std.fs";
pub fnc main(): int { return fs.read_text(1); }"#, "E_UNBOUND"),
        (r#"import * as fs from "std.fs";
pub fnc main(): int { return fs.read_file(1); }"#, "E_UNBOUND"),
        (r#"import * as fs from "std.fs";
pub fnc main(): int { return fs.open(1); }"#, "E_UNBOUND"),
        (r#"import * as fs from "std.fs";
pub fnc main(): int { return fs.read_stream(1); }"#, "E_UNBOUND"),
        (r#"import * as x from pkg "std.http";
import * as x from pkg "std.fs";
pub fnc main(): int { return 0; }"#, "E_DUPLICATE"),
        (r#"import * as http from pkg "std.http";
pub fnc __atto_file_import_0(): int {return 0;}
pub fnc main(): int {return 42;}"#, "E_RESERVED"),
        (r#"import * as http from pkg "std.http";
pub fnc main(): int { return http.missing(1); }"#, "E_UNBOUND"),
    ] {
        assert_eq!(code(source), expected, "{source}");
    }
}

#[test]
fn untrusted_relative_source_remains_unavailable_in_browser_compiler() {
    let source = r#"import * as other from "./stdlib/std/http_response_policy.atto";
pub fnc main(): int { return other.http_status_valid(200); }"#;
    let err: Diagnostic = compile(source).expect_err("browser must have no file read");
    assert_eq!(err.code, "E_IMPORT");
}


#[test]
fn local_file_with_same_virtual_path_cannot_alias_sealed_std_package() {
    let source = r#"
import * as http from pkg "std.http";
import * as local from "./stdlib/std/http_response_policy.atto";
pub fnc main(): int {
    if http.http_status_valid(200) == false then return 1; fi
    return local.answer();
}"#;
    let mut read_count = 0usize;
    let wasm = compile_with_imports(source, Path::new("main.atto"), |path| {
        assert_eq!(path, Path::new("stdlib/std/http_response_policy.atto"));
        read_count += 1;
        Ok("pub fnc answer(): int { return 42; }".to_owned())
    }).expect("local import and sealed std must be distinct source identities");
    assert_eq!(read_count, 1);
    assert!(wasm.starts_with(b"\0asm"));
}

#[test]
fn same_sealed_package_bound_to_two_aliases_reuses_one_source_without_collisions() {
    let source = r#"
import * as a from pkg "std.http";
import * as b from pkg "std.http";
pub fnc main(): int {
    if a.http_status_valid(200) == false then return 1; fi
    if b.http_status_valid(600) then return 2; fi
    return 42;
}"#;
    let wasm = compile(source).expect("repeated sealed import should de-duplicate identical unit");
    assert!(wasm.starts_with(b"\0asm"));
}


#[test]
fn namespace_first_std_imports_compile_and_bad_headers_fail_closed() {
    let source = r#"
use namespace storefront;
import * as http from pkg "std.http";
import * as fs from pkg "std.fs";
pub fnc main(): int {
    if http.http_status_valid(200) == false then return 1; fi
    if fs.fs_text_size_admissible(3, 3, 3) == false then return 2; fi
    return 42;
}"#;
    let bytes = compile(source).expect("namespace-first sealed std imports");
    assert!(bytes.starts_with(b"\0asm"));
    assert_eq!(bytes, compile(source).unwrap(), "deterministic namespace-first compilation");

    let derived = source.replace("use namespace storefront;", "use default namespace;");
    let bytes = atto_compiler::compile_with_path(
        &derived, Path::new("storefront/main.atto"))
        .expect("default namespace from logical package source path");
    assert!(bytes.starts_with(b"\0asm"));

    for (bad, expected) in [
        ("use namespace storefront; use namespace other; pub fnc main(): int { return 42; }", "E_DUPLICATE"),
        ("use namespace storefront; import * as http from pkg \"std.http\"; use namespace other; pub fnc main(): int { return 42; }", "E_DUPLICATE"),
        ("use default namespace(2); import * as http from pkg \"std.http\"; pub fnc main(): int { return 42; }", "E_NAMESPACE_DEPTH"),
    ] {
        let err = atto_compiler::compile_with_path(bad, Path::new("storefront/main.atto"))
            .expect_err("invalid namespace metadata may not create executable Wasm");
        assert_eq!(err.code, expected, "{bad}: {err}");
    }
}


#[test]
fn standalone_default_namespace_keeps_original_missing_path_diagnostic() {
    let source = "use default namespace; pub fnc main(): int { return 42; }";
    let err = compile(source).expect_err("missing logical folder must not be guessed");
    assert_eq!(err.code, "E_NAMESPACE_PATH");

    let ordinary = "use namespace storefront; pub fnc main(): int { return 42; }";
    // No new import path or ambient package is needed for this pre-existing
    // namespace-only source admission. Keep it on the ordinary compiler path.
    let outcome = compile(ordinary);
    if let Err(err) = outcome {
        assert_ne!(err.code, "E_IMPORT", "standalone namespace must not trigger source loading");
    }
}


#[test]
fn canonical_builtin_std_fs_links_compiler_owned_atto_api_and_io_rules() {
    let source = r#"
import fs_open_options_valid as options_ok, fs_io_next_chunk as next,
       fs_copy_fits_budget as copy_fits from "std.fs";
pub fnc main(): int {
    if options_ok(true, false, false, false, false, false) == false then return 1; fi
    if next(10, 4, 8) != 4 then return 2; fi
    if copy_fits(4, 2, 6, 9, 10) then return 3; fi
    return 42;
}
"#;
    let generated = compile(source).expect("compiler-owned std.fs .atto policy must compile");
    assert!(generated.starts_with(b"\0asm\x01\0\0\0"));
    assert!(!generated.windows(b"atto_fs_v1".len()).any(|w| w == b"atto_fs_v1"));

    // The built-in never consults the external project file loader.
    let mut reads = 0;
    let linked = compile_with_imports(source, Path::new("src/main.atto"), |_| {
        reads += 1;
        Err(std::io::Error::new(std::io::ErrorKind::PermissionDenied,
            "built-in may not load guest files"))
    }).expect("only compiler-embedded policy sources are needed");
    assert_eq!(reads, 0);
    assert_eq!(generated, linked);
}

#[test]
fn missing_builtin_modules_reject_without_invoking_project_source_loader() {
    for target in ["core.result", "alloc.arena", "std.unregistered"] {
        let source = format!(
            "import * as missing from \"{target}\"; pub fnc main(): int {{ return 42; }}"
        );
        let mut reads = 0;
        let err = compile_with_imports(&source, Path::new("src/main.atto"), |_| {
            reads += 1;
            Err(std::io::Error::new(std::io::ErrorKind::Other, "no ambient files"))
        }).expect_err("unimplemented built-in must produce no executable");
        assert_eq!(err.code, "E_STDLIB_UNLINKED", "{target}: {err}");
        assert_eq!(reads, 0, "{target} must fail before host loader");
    }
}

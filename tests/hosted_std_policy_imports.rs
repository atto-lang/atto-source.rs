//! Compiled Wasm via the production compiler, not a regex or source hash pass.
//! Execution (main() == 42) is in hosted_io_pure_policy_guest.mjs.
//! These sealed *pure-policy* std packages do not implement network or file I/O.
use atto_compiler::{compile, compile_with_imports, Diagnostic};
use std::path::Path;

const POSITIVE: &str = r#"
import * as http from pkg "std.http";
import * as fs from pkg "std.fs";
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

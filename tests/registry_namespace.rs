use atto_compiler::{package_resolver::SnapshotSource, registry_namespace::FrozenRegistry,
    sha256::hex_digest, static_import_graph::Scope, import_v2::Kind};

const MANIFEST: &str = r#"[package]
name = "@org/project/package"
version = "1.0.0"
[namespaces]
http = "src/http/"
http3 = "src/http3/"
c = "src/c/"
[exports]
"a/b/c" = "src/c/"
"#;
const HTTP: &str = "use namespace http; pub fnc get(int x): int { return x+1; } fnc secret(int x): int { return x; }";
const HTTP3: &str = "use namespace http3; pub fnc get(int x): int { return x+2; }";
const C: &str = "use namespace c; pub fnc get(int x): int { return x+3; }";
const INPUTS: [SnapshotSource<'static>; 3] = [
    SnapshotSource { path:"src/http/main.atto", source:HTTP },
    SnapshotSource { path:"src/http3/main.atto", source:HTTP3 },
    SnapshotSource { path:"src/c/main.atto", source:C },
];
fn lock(manifest: &str, inputs: &[SnapshotSource<'_>]) -> String {
    let mut s = format!("[lock]\nmanifest_sha256 = \"{}\"\n[files]\n",hex_digest(manifest.as_bytes()));
    for input in inputs {
        s.push_str(&format!("\"{}\" = \"{}\"\n",input.path,hex_digest(input.source.as_bytes())));
    }
    s
}
fn verify(manifest: &str, inputs: &[SnapshotSource<'_>]) -> Result<FrozenRegistry,atto_compiler::Diagnostic> {
    let l = lock(manifest,inputs);
    FrozenRegistry::verify("@org/project/package",manifest,&l,&hex_digest(l.as_bytes()),inputs)
}
fn error(r: Result<FrozenRegistry,atto_compiler::Diagnostic>) -> &'static str { r.err().expect("must reject").code }

#[test]
fn authenticated_external_package_suffixes_and_aliases_preserve_identity() {
    let p = verify(MANIFEST,&INPUTS).unwrap();
    let mut scope = Scope::new();
    p.bind_package(&mut scope,None).unwrap();
    for prefix in ["package","project.package","org.project.package"] {
        let t = scope.resolve_export(&format!("{prefix}.http.get"),p.view()).unwrap();
        assert_eq!(t.declaration,"http.get");
        assert_eq!(t.kind,Kind::Fnc);
    }
    scope.namespace_aliases(&[("first","second"),("second","org.project.package.http")],&[p.view()]).unwrap();
    assert_eq!(scope.resolve_export("first.get",p.view()).unwrap(),scope.resolve_export("package.http.get",p.view()).unwrap());
    assert_eq!(scope.resolve_export("first.secret",p.view()).unwrap_err().code,"E_PRIVATE");
}
#[test]
fn manifest_selection_and_subpaths_are_static_and_explicit() {
    let p = verify(MANIFEST,&INPUTS).unwrap();
    let mut scope = Scope::new();
    p.bind_namespace(&mut scope,"@org/project/package","http","http",false).unwrap();
    p.bind_namespace(&mut scope,"@org/project/package","vendor.http3","vendor.http3",false).unwrap();
    p.bind_namespace(&mut scope,"@org/project/package/a/b/c","custom","custom",false).unwrap();
    p.bind_namespace(&mut scope,"@org/project/package","package","root",true).unwrap();
    for (name,decl) in [("http.get","http.get"),("vendor.http3.get","http3.get"),("custom.get","c.get"),("root.c.get","c.get")] {
        assert_eq!(scope.resolve_export(name,p.view()).unwrap().declaration,decl);
    }
    assert_eq!(p.bind_namespace(&mut scope,"@org/project/package","http","invented",false).unwrap_err().code,"E_IMPORT_PATH");
    assert_eq!(p.bind_namespace(&mut scope,"@org/project/package/internal","c","c",false).unwrap_err().code,"E_UNBOUND");
    for source in ["@org/project/package/a/../c","@org/project/package/a//c","@org/project/package/%2e%2e/c","@other/project/package","https://example.test"] {
        assert_eq!(p.bind_namespace(&mut scope,source,"c","c",false).unwrap_err().code,"E_IMPORT_PATH");
    }
}
#[test]
fn source_lock_rejects_drift_missing_extra_duplicate_and_wrong_identity() {
    let l = lock(MANIFEST,&INPUTS);
    let h = hex_digest(l.as_bytes());
    assert_eq!(error(FrozenRegistry::verify("@wrong/project/package",MANIFEST,&l,&h,&INPUTS)),"E_LOCK_HASH");
    assert_eq!(error(FrozenRegistry::verify("@org/project/package",MANIFEST,&l,&"a".repeat(64),&INPUTS)),"E_LOCK_HASH");
    assert_eq!(error(FrozenRegistry::verify("@org/project/package",&format!("{MANIFEST}\n# changed"),&l,&h,&INPUTS)),"E_LOCK_HASH");
    let mut changed = INPUTS;
    changed[0].source = HTTP3;
    assert_eq!(error(FrozenRegistry::verify("@org/project/package",MANIFEST,&l,&h,&changed)),"E_LOCK_HASH");
    assert_eq!(error(FrozenRegistry::verify("@org/project/package",MANIFEST,&l,&h,&INPUTS[..2])),"E_LOCK_HASH");
    let duplicate = [INPUTS[0],INPUTS[0],INPUTS[2]];
    assert_eq!(error(FrozenRegistry::verify("@org/project/package",MANIFEST,&l,&h,&duplicate)),"E_DUPLICATE");
}
#[test]
fn authenticated_malformed_manifest_or_source_still_fails_closed() {
    for m in [
        MANIFEST.replace("src/http/","src/../http/"),
        MANIFEST.replace("src/http/","src//http/"),
        MANIFEST.replace("src/http/","src/http3/"),
        MANIFEST.replace("\"a/b/c\"","\"a/../c\""),
        MANIFEST.replace("\"a/b/c\" = \"src/c/\"","\"a/b/c\" = \"src/hidden/\""),
        format!("{MANIFEST}\n[exports]\n"),
    ] { assert!(verify(&m,&INPUTS).is_err(),"{m}"); }
    let mut changed = INPUTS;
    changed[0].source = HTTP3;
    assert_eq!(error(verify(MANIFEST,&changed)),"E_MANIFEST");
    changed[0].source = "use namespace http; import f from './hidden.atto'; pub fnc get(int x): int { return x; }";
    assert!(verify(MANIFEST,&changed).is_err());
    changed[0].source = "use namespace http; pub fnc __atto_fake(int x): int { return x; }";
    assert_eq!(error(verify(MANIFEST,&changed)),"E_RESERVED");
}
#[test]
fn distinct_locked_packages_do_not_collapse_and_alias_cycles_are_atomic() {
    let p = verify(MANIFEST,&INPUTS).unwrap();
    let mut changed = INPUTS;
    changed[0].source = "use namespace http; pub fnc get(int x): int { return x+5; }";
    let q = verify(MANIFEST,&changed).unwrap();
    let mut scope = Scope::new();
    p.bind_package(&mut scope,Some("p")).unwrap();
    q.bind_package(&mut scope,Some("q")).unwrap();
    assert_ne!(scope.resolve("p").unwrap().target,scope.resolve("q").unwrap().target);
    assert_eq!(scope.resolve_export("p.http.get",q.view()).unwrap_err().code,"E_IMPORT_PATH");
    assert_eq!(scope.namespace_aliases(&[("a","b"),("b","a")],&[p.view(),q.view()]).unwrap_err().code,"E_IMPORT_CYCLE");
    assert_eq!(scope.resolve("a").unwrap_err().code,"E_UNBOUND");
}

#[test]
fn external_source_alias_spellings_share_checked_identity_and_cycle_detection() {
    use atto_compiler::registry_namespace::bind_registry_aliases;
    let p = verify(MANIFEST,&INPUTS).unwrap();
    let mut scope = Scope::new();
    p.bind_package(&mut scope,Some("vendor")).unwrap();
    bind_registry_aliases(&mut scope,
        "define alias first = second; define alias vendor.http as second; define alias first as third;",
        &[&p]).unwrap();
    let expected = scope.resolve_export("vendor.http.get",p.view()).unwrap();
    for reference in ["first.get","second.get","third.get"] {
        assert_eq!(scope.resolve_export(reference,p.view()).unwrap(),expected);
    }
    assert_eq!(scope.resolve_export("third.secret",p.view()).unwrap_err().code,"E_PRIVATE");
    assert_eq!(bind_registry_aliases(&mut scope,
        "define alias a as b; define alias b as c; define alias c as a;",
        &[&p]).unwrap_err().code,"E_IMPORT_CYCLE");
    assert_eq!(scope.resolve("a").unwrap_err().code,"E_UNBOUND");
    assert_eq!(bind_registry_aliases(&mut scope,
        "define alias ok = vendor.http; define alias bad = vendor.http.secret;",
        &[&p]).unwrap_err().code,"E_PRIVATE");
    assert_eq!(scope.resolve("ok").unwrap_err().code,"E_UNBOUND");
}

#[test]
fn registry_guest_real_compiler_accepts_aliases_and_typed_functions() {
    use atto_compiler::registry_namespace::compile_registry_guest;
    let p=verify(MANIFEST,&INPUTS).unwrap();
    let sources=[
        "import pkg '@org/project/package'; pub fnc main(): int { return package.http.get(41); }",
        "import pkg '@org/project/package' as vendor; define alias first = second; define alias vendor.http as second; pub fnc main(): int { return first.get(41); }",
        "import fnc [http.get as fetch], namespace [http3] from pkg '@org/project/package'; pub fnc main(): int { return fetch(19)+http3.get(20); }",
        "import namespace custom from pkg '@org/project/package/a/b/c'; pub fnc main(): int { return custom.get(39); }",
    ];
    for (i,source) in sources.iter().enumerate() {
        let wasm=compile_registry_guest(source,&[&p]).unwrap();
        assert!(wasm.starts_with(b"\0asm"));
        assert_eq!(wasm,compile_registry_guest(source,&[&p]).unwrap());
        if let Ok(dir)=std::env::var("ATTO_REGISTRY_GUEST_DIR") {
            std::fs::write(std::path::Path::new(&dir).join(format!("registry-{i}.wasm")),wasm).unwrap();
        }
    }
}
#[test]
fn distinct_registry_packages_link_without_ambient_identity_capture() {
    use atto_compiler::registry_namespace::compile_registry_guest;
    let p=verify(MANIFEST,&INPUTS).unwrap();
    let other=MANIFEST.replace("@org/project/package","@other/project/package");
    let mut inputs=INPUTS;
    inputs[0].source="use namespace http; pub fnc get(int x): int { return x+5; }";
    let l=lock(&other,&inputs);
    let q=FrozenRegistry::verify("@other/project/package",&other,&l,&hex_digest(l.as_bytes()),&inputs).unwrap();
    let source="import pkg '@org/project/package'; import pkg '@other/project/package'; pub fnc main(): int { return org.project.package.http.get(18)+other.project.package.http.get(18); }";
    let wasm=compile_registry_guest(source,&[&p,&q]).unwrap();
    assert_eq!(wasm,compile_registry_guest(source,&[&q,&p]).unwrap());
    if let Ok(dir)=std::env::var("ATTO_REGISTRY_GUEST_DIR") {
        std::fs::write(std::path::Path::new(&dir).join("registry-distinct.wasm"),wasm).unwrap();
    }
    let ambiguous="import pkg '@org/project/package'; import pkg '@other/project/package'; pub fnc main(): int { return package.http.get(41); }";
    assert_eq!(compile_registry_guest(ambiguous,&[&p,&q]).unwrap_err().code,"E_AMBIGUOUS");
}
#[test]
fn registry_guest_rejects_private_unbound_forged_and_cycle_references() {
    use atto_compiler::registry_namespace::compile_registry_guest;
    let p=verify(MANIFEST,&INPUTS).unwrap();
    for (source,code) in [
        ("import pkg '@org/project/package'; pub fnc main(): int { return package.http.secret(42); }","E_PRIVATE"),
        ("import pkg '@org/project/package'; pub fnc main(): int { return package.http.absent(42); }","E_UNBOUND"),
        ("import pkg '@org/project/package' as vendor; pub fnc main(): int { return package.http.get(41); }","E_UNBOUND"),
        ("import pkg '@org/project/package'; pub fnc main(): int { return __atto_registry_0.get(41); }","E_RESERVED"),
        ("import pkg '@org/project/package'; define alias a as b; define alias b as c; define alias c as a; pub fnc main(): int { return 42; }","E_IMPORT_CYCLE"),
        ("import pkg '@org/project/package'; pub fnc main(): int { return package; }","E_UNSUPPORTED"),
        ("import pkg '@unknown/project/package'; pub fnc main(): int { return 42; }","E_UNBOUND"),
    ] {
        assert_eq!(compile_registry_guest(source,&[&p]).unwrap_err().code,code,"{source}");
    }
    let mut changed=INPUTS;
    changed[0].source="use namespace http; pub fnc get(int x): int { return capture(x); }";
    let q=verify(MANIFEST,&changed).unwrap();
    let source="import pkg '@org/project/package'; fnc capture(int x): int { return 42; } pub fnc main(): int { return package.http.get(1); }";
    assert_eq!(compile_registry_guest(source,&[&q]).unwrap_err().code,"E_UNSUPPORTED");
}

#[test]
fn registry_tree_guest_executes_and_source_admission_is_order_independent(){
    use atto_compiler::registry_namespace::compile_registry_guest;
    let p=verify(MANIFEST,&INPUTS).unwrap();
    let sources=[
        "import pkg '@org/project/package' as p; import p.{http.{fnc get as fetch}, http3.{self as h, get as other},}; pub fnc main(): int { return fetch(19)+h.get(10)+other(8); }",
        "import p.{http.{fnc get as fetch}, http3.{self as h, get as other},}; import pkg '@org/project/package' as p; pub fnc main(): int { return fetch(19)+h.get(10)+other(8); }",
        "import pkg '@org/project/package' as p; import fnc p.http.get as fetch; pub fnc main(): int { return fetch(41); }",
        "import pkg '@org/project/package' as p; import p.http.{self as h}; define alias h as math; pub fnc main(): int { return math.get(41); }",
    ];
    let mut bytes=Vec::new();
    for (i,source) in sources.iter().enumerate(){
        let wasm=compile_registry_guest(source,&[&p]).unwrap();
        assert!(wasm.starts_with(b"\0asm"));
        if let Ok(dir)=std::env::var("ATTO_REGISTRY_GUEST_DIR"){
            std::fs::write(std::path::Path::new(&dir).join(format!("registry-tree-{i}.wasm")),&wasm).unwrap();
        }
        bytes.push(wasm);
    }
    assert_eq!(bytes[0],bytes[1]);
}
#[test]
fn registry_tree_guest_denies_private_missing_and_wrong_kind(){
    use atto_compiler::registry_namespace::compile_registry_guest;
    let p=verify(MANIFEST,&INPUTS).unwrap();
    for (clause,code) in [
        ("import p.http.{get, secret};","E_PRIVATE"),
        ("import p.http.{get, missing};","E_UNBOUND"),
        ("import p.http.{class get};","E_IMPORT_KIND"),
        ("import p.http.*;","E_SYNTAX"),
        ("import p.http.{get as x, secret as x};","E_DUPLICATE"),
        ("import p.http.{};","E_SYNTAX"),
    ]{
        let source=format!("import pkg '@org/project/package' as p; {clause} pub fnc main(): int {{ return 42; }}");
        assert_eq!(compile_registry_guest(&source,&[&p]).unwrap_err().code,code,"{source}");
    }
}

#[test]
fn contextual_guest_resolves_current_parent_and_grandparent_without_ambient_authority(){
    use atto_compiler::registry_namespace::compile_contextual_guest;
    let manifest=r#"[package]
name = "@org/project/package"
version = "1.0.0"
[namespaces]
network = "src/network/"
"network.http" = "src/http/"
"network.http.utils" = "src/utils/"
"network.config" = "src/config/"
common = "src/common/"
"#;
    let inputs=[
        SnapshotSource{path:"src/network/main.atto",source:"use namespace network; pub fnc get(): int { return 42; }"},
        SnapshotSource{path:"src/http/main.atto",source:"use namespace network.http; pub fnc get(): int { return 42; } fnc secret(): int { return 42; }"},
        SnapshotSource{path:"src/utils/main.atto",source:"use namespace network.http.utils; pub fnc get(): int { return 42; }"},
        SnapshotSource{path:"src/config/main.atto",source:"use namespace network.config; pub fnc get(): int { return 42; }"},
        SnapshotSource{path:"src/common/main.atto",source:"use namespace common; pub fnc get(): int { return 42; }"},
    ];
    let p=verify(manifest,&inputs).unwrap();
    for (i,source) in [
        "import pkg.network.http; pub fnc main(): int { return http.get(); }",
        "use namespace network.http; import self.utils; pub fnc main(): int { return utils.get(); }",
        "use namespace network.http; import super.config as cfg; pub fnc main(): int { return cfg.get(); }",
        "use namespace network.http; import super.super.common; pub fnc main(): int { return common.get(); }",
        "use namespace network.http; import self.{utils as u, get as answer}; pub fnc main(): int { return answer(); }",
        "use default namespace(2); import self.utils; pub fnc main(): int { return utils.get(); }",
    ].iter().enumerate(){
        let wasm=compile_contextual_guest(source,&p,&[],Some(std::path::Path::new("network/http/app.atto"))).unwrap();
        assert!(wasm.starts_with(b"\0asm"));
        if let Ok(dir)=std::env::var("ATTO_REGISTRY_GUEST_DIR"){
            std::fs::write(std::path::Path::new(&dir).join(format!("registry-context-{i}.wasm")),wasm).unwrap();
        }
    }
    for (source,code) in [
        ("import self.utils; pub fnc main(): int { return 42; }","E_IMPORT_PATH"),
        ("use namespace network.http; import super.super.super.common; pub fnc main(): int { return 42; }","E_IMPORT_PATH"),
        ("use namespace forged; import self.utils; pub fnc main(): int { return 42; }","E_IMPORT_PATH"),
        ("use namespace network.http; import self.secret; pub fnc main(): int { return secret(); }","E_PRIVATE"),
        ("use namespace network.http; import self.{get as answer, secret}; pub fnc main(): int { return answer(); }","E_PRIVATE"),
    ]{assert_eq!(compile_contextual_guest(source,&p,&[],None).unwrap_err().code,code,"{source}");}
    assert!(compile_contextual_guest("import pkg.network.http; pub fnc main(): int { return http.get(); }",&p,&[&p],None).is_err());
}

#[test]
fn frozen_registry_literals_link_with_functions_and_aliases(){
    use atto_compiler::registry_namespace::compile_registry_guest;
    let manifest=MANIFEST.replace("[exports]","limits = \"src/limits/\"\n[exports]");
    let mut inputs=INPUTS.to_vec();
    inputs.push(SnapshotSource{path:"src/limits/main.atto",source:"use namespace limits; define pub const int MAX = 41; define pub const bool READY = true; define pub const int LOW = -9223372036854775808; define const int SECRET = 99;"});
    let p=verify(&manifest,&inputs).unwrap();
    for (i,source) in [
        "import pkg '@org/project/package' as p; pub fnc main(): int { return p.http.get(p.limits.MAX); }",
        "import const limits.MAX as cap from pkg '@org/project/package'; pub fnc main(): int { return cap+1; }",
        "import [const limits.MAX as cap, fnc http.get as inc] from pkg '@org/project/package'; pub fnc main(): int { return inc(cap); }",
        "import pkg '@org/project/package' as p; import p.limits.{const MAX as cap, const READY}; pub fnc main(): int { if READY then return cap+1; fi return 0; }",
        "import pkg '@org/project/package' as p; define alias p.limits as l; pub fnc main(): int { if l.LOW < 0 then return l.MAX+1; fi return 0; }",
    ].iter().enumerate(){
        let wasm=compile_registry_guest(source,&[&p]).unwrap();
        assert!(wasm.starts_with(b"\0asm"));
        if let Ok(dir)=std::env::var("ATTO_REGISTRY_GUEST_DIR"){
            std::fs::write(std::path::Path::new(&dir).join(format!("registry-const-{i}.wasm")),wasm).unwrap();
        }
    }
    for (source,code) in [
        ("import pkg '@org/project/package' as p; pub fnc main(): int { return p.limits.SECRET; }","E_PRIVATE"),
        ("import const limits.MAX as cap from pkg '@org/project/package'; pub fnc main(): int { cap = 42; return cap; }","E_ASSIGN_CONST"),
        ("import const limits.MAX as cap from pkg '@org/project/package'; pub fnc main(): int { return cap(); }","E_IMPORT_KIND"),
        ("import const limits.MAX as cap from pkg '@org/project/package'; pub fnc main(): int { let int cap = 42; return cap; }","E_ASSIGN_CONST"),
        ("import pkg '@org/project/package' as p; define alias p.limits.MAX as a; pub fnc main(): int { return 42; }","E_IMPORT_KIND"),
    ]{assert_eq!(compile_registry_guest(source,&[&p]).unwrap_err().code,code,"{source}");}
    for bad in [
        "use namespace limits; define pub const int MAX = true;",
        "use namespace limits; define pub const int MAX = 9223372036854775808;",
        "use namespace limits; define pub const int MAX = std.env.read_env('X');",
        "use namespace limits; define pub const int MAX = 41; pub fnc MAX(): int { return 1; }",
    ]{
        inputs.last_mut().unwrap().source=bad;
        assert!(verify(&manifest,&inputs).is_err(),"{bad}");
    }
}

#[test]
fn mixed_registry_provider_uses_private_literals_without_exporting_them() {
    use atto_compiler::registry_namespace::compile_registry_guest;
    let mut inputs=INPUTS;
    inputs[0].source="use namespace http; pub fnc get(int x): int { return x+OFFSET; } define const int OFFSET = 1; define pub const int LIMIT = 41; pub fnc answer(): int { return get(LIMIT); }";
    let p=verify(MANIFEST,&inputs).unwrap();
    for (i,source) in [
        "import pkg '@org/project/package' as p; pub fnc main(): int { return p.http.get(p.http.LIMIT); }",
        "import [const http.LIMIT as cap, fnc http.get as inc] from pkg '@org/project/package'; pub fnc main(): int { return inc(cap); }",
        "import pkg '@org/project/package' as p; import p.http.{self as h, fnc answer}; pub fnc main(): int { return answer(); }",
    ].iter().enumerate() {
        let wasm=compile_registry_guest(source,&[&p]).unwrap();
        assert!(wasm.starts_with(b"\0asm"));
        if let Ok(dir)=std::env::var("ATTO_REGISTRY_GUEST_DIR"){
            std::fs::write(std::path::Path::new(&dir).join(format!("registry-mixed-{i}.wasm")),wasm).unwrap();
        }
    }
    assert_eq!(compile_registry_guest("import pkg '@org/project/package' as p; pub fnc main(): int { return p.http.OFFSET; }",&[&p]).unwrap_err().code,"E_PRIVATE");
    for bad in [
        "use namespace http; define const int OFFSET = 1; pub fnc get(int OFFSET): int { return OFFSET; }",
        "use namespace http; define const int OFFSET = 1; pub fnc get(int x): int { OFFSET = x; return OFFSET; }",
        "use namespace http; define const int OFFSET = 1; pub fnc get(int x): int { return OFFSET(); }",
        "use namespace http; define const int OFFSET = 1; pub fnc OFFSET(): int { return 42; }",
        "use namespace http; pub fnc get(int x): int { return x; } define const int OFFSET = std.env.read_env('X');",
    ]{
        inputs[0].source=bad;
        assert!(verify(MANIFEST,&inputs).is_err(),"{bad}");
    }
}

#[test]
fn pinned_multifile_namespace_links_public_members_and_preserves_private_ownership() {
    use atto_compiler::registry_namespace::compile_registry_guest;
    let mut inputs=INPUTS.to_vec();
    inputs[0].source="use namespace http; pub fnc get(int x): int { return inc(x); } pub fnc answer(): int { return get(LIMIT); }";
    inputs.push(SnapshotSource{path:"src/http/extra.atto",source:"use namespace http; define pub const int LIMIT = 41; define const int OFFSET = 1; pub fnc inc(int x): int { return x+OFFSET; }"});
    let l=lock(MANIFEST,&inputs);
    let digest=hex_digest(l.as_bytes());
    let p=FrozenRegistry::verify("@org/project/package",MANIFEST,&l,&digest,&inputs).unwrap();
    let mut reversed=inputs.clone();
    reversed.reverse();
    let q=FrozenRegistry::verify("@org/project/package",MANIFEST,&l,&digest,&reversed).unwrap();
    for (i,source) in [
        "import pkg '@org/project/package' as p; pub fnc main(): int { return p.http.answer(); }",
        "import [const http.LIMIT as cap, fnc http.get as inc] from pkg '@org/project/package'; pub fnc main(): int { return inc(cap); }",
        "import namespace http from pkg '@org/project/package'; define alias http as h; pub fnc main(): int { return h.get(h.LIMIT); }",
    ].iter().enumerate(){
        let wasm=compile_registry_guest(source,&[&p]).unwrap();
        assert!(wasm.starts_with(b"\0asm"));
        assert_eq!(wasm,compile_registry_guest(source,&[&q]).unwrap());
        if let Ok(dir)=std::env::var("ATTO_REGISTRY_GUEST_DIR"){
            std::fs::write(std::path::Path::new(&dir).join(format!("registry-multifile-{i}.wasm")),wasm).unwrap();
        }
    }
    assert_eq!(compile_registry_guest("import pkg '@org/project/package' as p; pub fnc main(): int { return p.http.OFFSET; }",&[&p]).unwrap_err().code,"E_PRIVATE");
    assert_eq!(error(FrozenRegistry::verify("@org/project/package",MANIFEST,&l,&digest,&inputs[..3])),"E_LOCK_HASH");
    for (bad,code) in [
        ("use namespace http; pub fnc inc(int x): int { return x; }","E_DUPLICATE"),
        ("use namespace wrong; pub fnc get(int x): int { return x; }","E_MANIFEST"),
    ]{
        let mut changed=inputs.clone();
        changed[0].source=bad;
        assert_eq!(error(verify(MANIFEST,&changed)),code,"{bad}");
    }
    let mut changed=inputs.clone();
    changed[0].source="use namespace http; pub fnc get(int x): int { return x;";
    assert!(verify(MANIFEST,&changed).is_err());
    changed=inputs.clone();
    changed.last_mut().unwrap().path="src/http/nested/extra.atto";
    assert_eq!(error(verify(MANIFEST,&changed)),"E_UNSUPPORTED");
}

#[test]
fn physical_declaration_provenance_never_grants_private_authority() {
    use atto_compiler::registry_namespace::{compile_registry_guest,compile_contextual_guest};
    let mut inputs=INPUTS.to_vec();
    inputs[0].source="use namespace http; fnc secret(int x): int { return x+1; } pub fnc get(int x): int { return secret(x); }";
    inputs.push(SnapshotSource{path:"src/http/extra.atto",source:"use namespace http; define pub const int LIMIT = 41; pub fnc answer(): int { return get(LIMIT); }"});
    let p=verify(MANIFEST,&inputs).unwrap();
    let mut scope=Scope::new();
    p.bind_package(&mut scope,Some("p")).unwrap();
    let get=scope.resolve_export("p.http.get",p.view()).unwrap();
    let limit=scope.resolve_export("p.http.LIMIT",p.view()).unwrap();
    assert_eq!(p.declaration_source(&get).unwrap(),"src/http/main.atto");
    assert_eq!(p.declaration_source(&limit).unwrap(),"src/http/extra.atto");
    let mut forged=get.clone();
    forged.kind=Kind::Const;
    assert_eq!(p.declaration_source(&forged).unwrap_err().code,"E_IMPORT_KIND");
    forged=get.clone();
    forged.declaration="http.secret".into();
    assert_eq!(p.declaration_source(&forged).unwrap_err().code,"E_PRIVATE");
    let source="import pkg '@org/project/package' as p; pub fnc main(): int { return p.http.answer(); }";
    let wasm=compile_registry_guest(source,&[&p]).unwrap();
    if let Ok(dir)=std::env::var("ATTO_REGISTRY_GUEST_DIR"){
        std::fs::write(std::path::Path::new(&dir).join("registry-owner-0.wasm"),wasm).unwrap();
    }
    let spoof="use namespace http; import self.secret; pub fnc main(): int { return secret(41); }";
    assert_eq!(compile_contextual_guest(spoof,&p,&[],Some(std::path::Path::new("src/http/main.atto"))).unwrap_err().code,"E_PRIVATE");
    let mut changed=inputs.clone();
    changed.last_mut().unwrap().source="use namespace http; pub fnc answer(): int { return secret(41); }";
    let same_namespace=verify(MANIFEST,&changed).unwrap();
    assert!(compile_registry_guest(source,&[&same_namespace]).is_ok());
    changed=inputs.clone();
    changed.last_mut().unwrap().source="use namespace http; define pub const int LIMIT = 40; pub fnc answer(): int { return get(LIMIT); }";
    let q=verify(MANIFEST,&changed).unwrap();
    assert_eq!(q.declaration_source(&get).unwrap_err().code,"E_IMPORT_PATH");
}

#[test]
fn pinned_namespace_private_members_cross_files_but_not_boundaries() {
    use atto_compiler::registry_namespace::{compile_registry_guest,compile_contextual_guest};
    let root="import pkg '@org/project/package' as p; pub fnc main(): int { return p.http.answer(); }";
    for (i,provider) in [
        "use namespace http; define const int OFFSET = 1; fnc secret(int x): int { return x+OFFSET; }",
        "use namespace http; fnc secret(int x): int { return x+1; }",
    ].iter().enumerate(){
        let mut inputs=INPUTS.to_vec();
        inputs[0].source=if i==0 {
            "use namespace http; pub fnc answer(): int { return secret(40)+OFFSET; }"
        }else{
            "use namespace http; pub fnc answer(): int { return secret(41); }"
        };
        inputs.push(SnapshotSource{path:"src/http/private.atto",source:provider});
        let l=lock(MANIFEST,&inputs);
        let digest=hex_digest(l.as_bytes());
        let p=FrozenRegistry::verify("@org/project/package",MANIFEST,&l,&digest,&inputs).unwrap();
        let wasm=compile_registry_guest(root,&[&p]).unwrap();
        assert!(wasm.starts_with(b"\0asm"));
        let mut reversed=inputs.clone();
        reversed.reverse();
        let q=FrozenRegistry::verify("@org/project/package",MANIFEST,&l,&digest,&reversed).unwrap();
        assert_eq!(wasm,compile_registry_guest(root,&[&q]).unwrap());
        if let Ok(dir)=std::env::var("ATTO_REGISTRY_GUEST_DIR"){
            std::fs::write(std::path::Path::new(&dir).join(format!("registry-private-{i}.wasm")),wasm).unwrap();
        }
        for bad in [
            "import pkg '@org/project/package' as p; pub fnc main(): int { return p.http.secret(41); }",
            "import pkg '@org/project/package' as p; define alias h = p.http; pub fnc main(): int { return h.secret(41); }",
        ]{
            assert_eq!(compile_registry_guest(bad,&[&p]).unwrap_err().code,"E_PRIVATE");
        }
        let spoof="use namespace http; import self.secret; pub fnc main(): int { return secret(41); }";
        assert_eq!(compile_contextual_guest(spoof,&p,&[],Some(std::path::Path::new("src/http/private.atto"))).unwrap_err().code,"E_PRIVATE");
        let mut changed=inputs.clone();
        changed[1].source="use namespace http3; pub fnc get(int x): int { return secret(x); }";
        assert!(compile_registry_guest(root,&[&verify(MANIFEST,&changed).unwrap()]).is_err());
        changed=inputs.clone();
        changed.last_mut().unwrap().source="use namespace http; fnc secret(int x): int { return 0; }";
        assert_eq!(error(FrozenRegistry::verify("@org/project/package",MANIFEST,&l,&digest,&changed)),"E_LOCK_HASH");
    }
}


#[test]
fn authenticated_single_class_guest_uses_constructor_and_method_checker() {
    use atto_compiler::registry_namespace::FrozenClass;
    let manifest="[package]\nname = \"@org/project/package\"\nversion = \"1.0.0\"\n[namespaces]\nmodels = \"src/models/\"\n";
    let source="use namespace models; define pub class Counter as let int value = 0; pub constructor(int x) { self.value = x; } pub fnc get(): int { return self.value+1; } fnc hidden(): int { return 42; } end";
    let inputs=[SnapshotSource{path:"src/models/counter.atto",source}];
    let l=lock(manifest,&inputs);
    let digest=hex_digest(l.as_bytes());
    let p=FrozenClass::verify("@org/project/package",manifest,&l,&digest,&inputs).unwrap();
    let prefix="import [class models.Counter] from pkg '@org/project/package'; ";
    let guest=format!("{prefix}pub fnc main(): int {{ val c = new Counter(41); return c.get(); }}");
    let wasm=p.compile_guest(&guest).unwrap();
    assert!(wasm.starts_with(b"\0asm"));
    if let Ok(dir)=std::env::var("ATTO_REGISTRY_GUEST_DIR"){
        std::fs::write(std::path::Path::new(&dir).join("registry-class-0.wasm"),wasm).unwrap();
    }
    for bad in [
        format!("{prefix}pub fnc main(): int {{ val c = new Counter(); return c.get(); }}"),
        format!("{prefix}pub fnc main(): int {{ val c = new Counter(41); return c.hidden(); }}"),
        format!("{prefix}pub fnc main(): int {{ val c = new Counter(41); return c.value; }}"),
        "import [fnc models.Counter] from pkg '@org/project/package'; pub fnc main(): int { return 42; }".into(),
        "import [class models.Counter as C] from pkg '@org/project/package'; pub fnc main(): int { return 42; }".into(),
        "import [class models.Counter] from pkg '@other/project/package'; pub fnc main(): int { return 42; }".into(),
    ]{
        assert!(p.compile_guest(&bad).is_err(),"{bad}");
    }
    let tampered=[SnapshotSource{path:inputs[0].path,source:"use namespace models; define class Counter as end"}];
    assert_eq!(FrozenClass::verify("@org/project/package",manifest,&l,&digest,&tampered).err().unwrap().code,"E_LOCK_HASH");
    for replacement in [
        source.replace("define pub class","define class"),
        source.replace("pub constructor","constructor"),
        source.replace("use namespace models","use namespace other"),
        source.replace("self.value+1","main()"),
    ]{
        let changed=[SnapshotSource{path:inputs[0].path,source:&replacement}];
        let cl=lock(manifest,&changed);
        assert!(FrozenClass::verify("@org/project/package",manifest,&cl,&hex_digest(cl.as_bytes()),&changed).is_err());
    }
}


#[test]
fn frozen_class_aliases_preserve_distinct_package_types() {
    use atto_compiler::registry_namespace::{FrozenClass,compile_frozen_classes};
    let manifest_a="[package]\nname = \"@a/project/pkg\"\nversion = \"1.0.0\"\n[namespaces]\nmodels = \"src/models/\"\n";
    let manifest_b=manifest_a.replace("@a/project/pkg","@b/project/pkg");
    let body_a="use namespace models; define pub class Counter as let int value = 0; pub constructor(int x) { self.value = x; } pub fnc get(): int { return self.value+1; } pub static fnc tag(): int { return 20; } end";
    let body_b=body_a.replace("self.value+1","self.value+2").replace("return 20;","return 22;");
    let a=[SnapshotSource{path:"src/models/counter.atto",source:body_a}];
    let b=[SnapshotSource{path:"src/models/counter.atto",source:&body_b}];
    let la=lock(manifest_a,&a);
    let lb=lock(&manifest_b,&b);
    let pa=FrozenClass::verify("@a/project/pkg",manifest_a,&la,&hex_digest(la.as_bytes()),&a).unwrap();
    let pb=FrozenClass::verify("@b/project/pkg",&manifest_b,&lb,&hex_digest(lb.as_bytes()),&b).unwrap();
    let prefix="import [class models.Counter as A] from pkg '@a/project/pkg'; import [class models.Counter as B] from pkg '@b/project/pkg'; ";
    let guests=[
        format!("{prefix}pub fnc main(): int {{ val a = new A(10); val b = new B(29); return a.get()+b.get(); }}"),
        format!("{prefix}pub fnc main(): int {{ return A.tag()+B.tag(); }}"),
        "import [class models.Counter as A] from pkg '@a/project/pkg'; import [class models.Counter as Again] from pkg '@a/project/pkg'; pub fnc main(): int { val a: A = new Again(41); return a.get(); }".into(),
    ];
    for (i,guest) in guests.iter().enumerate() {
        let wasm=compile_frozen_classes(guest,&[&pa,&pb]).unwrap();
        assert!(wasm.starts_with(b"\0asm"));
        assert_eq!(wasm,compile_frozen_classes(guest,&[&pb,&pa]).unwrap());
        if let Ok(dir)=std::env::var("ATTO_REGISTRY_GUEST_DIR"){
            std::fs::write(std::path::Path::new(&dir).join(format!("registry-class-multi-{i}.wasm")),wasm).unwrap();
        }
    }
    for bad in [
        format!("{prefix}pub fnc main(): int {{ val a: A = new B(41); return a.get(); }}"),
        format!("{prefix}pub fnc main(): int {{ val a = new Counter(41); return a.get(); }}"),
        format!("{prefix}pub fnc main(): int {{ val A = 42; return A; }}"),
        format!("{prefix}pub fnc main(): int {{ val a = new __atto_frozen_class_0(41); return a.get(); }}"),
        "import [class models.Counter as A] from pkg '@a/project/pkg'; import [class models.Counter as A] from pkg '@b/project/pkg'; pub fnc main(): int { return 42; }".into(),
    ]{
        assert!(compile_frozen_classes(&bad,&[&pa,&pb]).is_err(),"{bad}");
    }
    assert_eq!(compile_frozen_classes(&guests[0],&[&pa,&pa,&pb]).unwrap_err().code,"E_DUPLICATE");
}


#[test]
fn authenticated_newable_factory_is_sealed_and_exact() {
    use atto_compiler::registry_namespace::FrozenClass;
    let manifest="[package]\nname = \"@org/project/package\"\nversion = \"1.0.0\"\n[namespaces]\nmodels = \"src/models/\"\n";
    let body="use namespace models; define pub class Cart implements Newable[Cart] as val int count = 42; pub fnc new(): Cart { return new Cart(); } pub get_count(): int { return self.count; } end";
    let inputs=[SnapshotSource{path:"src/models/cart.atto",source:body}];
    let l=lock(manifest,&inputs);
    let digest=hex_digest(l.as_bytes());
    let p=FrozenClass::verify_newable("@org/project/package",manifest,&l,&digest,&inputs).unwrap();
    let prefix="import [class models.Cart] from pkg '@org/project/package'; ";
    let guest=format!("{prefix}pub fnc main(): int {{ val c = Cart.new(); return c.get_count(); }}");
    let wasm=p.compile_guest(&guest).unwrap();
    assert!(wasm.starts_with(b"\0asm"));
    if let Ok(dir)=std::env::var("ATTO_REGISTRY_GUEST_DIR"){
        std::fs::write(std::path::Path::new(&dir).join("registry-newable-0.wasm"),wasm).unwrap();
    }
    for bad in [
        format!("{prefix}pub fnc main(): int {{ val c = new Cart(); return c.get_count(); }}"),
        format!("{prefix}pub fnc main(): int {{ val c = Cart.new(1); return c.get_count(); }}"),
        format!("{prefix}pub fnc main(): int {{ val c = Cart.__atto_sealed_newable_factory(); return c.get_count(); }}"),
    ]{
        assert!(p.compile_guest(&bad).is_err(),"{bad}");
    }
    assert!(FrozenClass::verify("@org/project/package",manifest,&l,&digest,&inputs).is_err());
    for changed in [
        body.replace("implements Newable[Cart] ",""),
        body.replace("Newable[Cart]","Newable[int]"),
        body.replace("new(): Cart","new(): int"),
        body.replace("pub fnc new","fnc new"),
        body.replace("new(): Cart","new(int x): Cart"),
        body.replace("return new Cart();","return new Other();"),
        body.replace("define pub class","define trait Newable as end define pub class"),
    ]{
        let inputs=[SnapshotSource{path:"src/models/cart.atto",source:&changed}];
        let l=lock(manifest,&inputs);
        assert!(FrozenClass::verify_newable("@org/project/package",manifest,&l,&hex_digest(l.as_bytes()),&inputs).is_err(),"{changed}");
    }
}

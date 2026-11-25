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
        ("use namespace http; pub fnc get(int x): int { return x+OFFSET; }","E_PRIVATE"),
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
    assert_eq!(error(verify(MANIFEST,&changed)),"E_PRIVATE");
    changed=inputs.clone();
    changed.last_mut().unwrap().source="use namespace http; define pub const int LIMIT = 40; pub fnc answer(): int { return get(LIMIT); }";
    let q=verify(MANIFEST,&changed).unwrap();
    assert_eq!(q.declaration_source(&get).unwrap_err().code,"E_IMPORT_PATH");
}

#[test]
fn authenticated_class_metadata_uses_source_visibility_and_nominal_identity() {
    use atto_compiler::registry_namespace::compile_registry_guest;
    let manifest=MANIFEST.replace("[exports]","model = \"src/model/\"\n[exports]");
    let mut inputs=INPUTS.to_vec();
    inputs.push(SnapshotSource{path:"src/model/types.atto",source:
        "use namespace model; pub define class Item as pub int value: 42; end define class Hidden as int value: 0; end"});
    let p=verify(&manifest,&inputs).unwrap();
    let mut scope=Scope::new();
    p.bind_package(&mut scope,Some("p")).unwrap();
    let target=scope.resolve_export("p.model.Item",p.view()).unwrap();
    assert_eq!(target.kind,Kind::Class);
    assert_eq!(p.declaration_source(&target).unwrap(),"src/model/types.atto");
    let leaves=atto_compiler::import_tree::parse(&atto_compiler::lexer::lex("import class p.model.Item as Data;").unwrap(),1).unwrap().0;
    atto_compiler::import_tree::bind(&mut scope,&leaves,&[p.view()]).unwrap();
    let identity=scope.resolve_nominal("Data",p.view()).unwrap();
    assert_eq!(identity,scope.resolve_nominal("p.model.Item",p.view()).unwrap());
    assert_eq!(scope.resolve_export("p.model.Hidden",p.view()).unwrap_err().code,"E_PRIVATE");
    let bad=atto_compiler::import_tree::parse(&atto_compiler::lexer::lex("import p.model.{class Item as Good, class Hidden};").unwrap(),1).unwrap().0;
    assert_eq!(atto_compiler::import_tree::bind(&mut scope,&bad,&[p.view()]).unwrap_err().code,"E_PRIVATE");
    assert_eq!(scope.resolve("Good").unwrap_err().code,"E_UNBOUND");
    let source="import pkg '@org/project/package' as p; pub fnc main(): int { return 42; }";
    assert_eq!(compile_registry_guest(source,&[&p]).unwrap_err().code,"E_UNSUPPORTED");
    let l=lock(&manifest,&inputs);
    let digest=hex_digest(l.as_bytes());
    inputs.last_mut().unwrap().source="use namespace model; pub define class Item as pub int value: 41; end";
    assert_eq!(error(FrozenRegistry::verify("@org/project/package",&manifest,&l,&digest,&inputs)),"E_LOCK_HASH");
    let q=verify(&manifest,&inputs).unwrap();
    let mut other=Scope::new();
    q.bind_package(&mut other,Some("q")).unwrap();
    assert_ne!(identity,other.resolve_nominal("q.model.Item",q.view()).unwrap());
}

#[test]
fn authenticated_class_metadata_rejects_invalid_and_mixed_units() {
    let manifest=MANIFEST.replace("[exports]","model = \"src/model/\"\n[exports]");
    let mut inputs=INPUTS.to_vec();
    inputs.push(SnapshotSource{path:"src/model/types.atto",source:""});
    for source in [
        "use namespace model; pub define class Item as pub int value: 42;",
        "use namespace model; pub define class Item as pub int value: true; end",
        "use namespace model; pub define class Item as pub int value: 42; end define class Item as int value: 0; end",
        "use namespace model; pub define class Item as pub int value: 42; end pub fnc f(): int { return 42; }",
        "use namespace model; pub define class Item as pub int value: 42; end define enum State as Ready end",
        "use namespace model; pub define class __atto_fake as int value: 0; end",
    ] {
        inputs.last_mut().unwrap().source=source;
        assert!(verify(&manifest,&inputs).is_err(),"{source}");
    }
}

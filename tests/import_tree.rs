use atto_compiler::{import_tree::{parse,bind,contextual_path}, import_v2::Kind, lexer::lex, static_import_graph::{Scope,ExportView,Origin,PackageId}};

fn leaves(source:&str)->Vec<atto_compiler::import_tree::Leaf>{
    parse(&lex(source).unwrap(),1).unwrap().0
}
fn graph()->(Scope,ExportView){
    let id=PackageId::new("@org/project/package","1.0.0",&"a".repeat(64)).unwrap();
    let mut view=ExportView::new(Origin::External(id.clone()));
    for (p,k,public) in [
        ("net",Kind::Namespace,true),("net.http",Kind::Namespace,true),
        ("net.http.get",Kind::Fnc,true),("net.http.Request",Kind::Class,true),
        ("net.http.secret",Kind::Fnc,false),("net.tcp",Kind::Namespace,true),
        ("net.tcp.connect",Kind::Fnc,true),
    ]{view.add(p,p,k,public).unwrap();}
    let mut scope=Scope::new();
    scope.package(id,Some("p")).unwrap();
    (scope,view)
}
#[test]
fn recursive_trees_and_self_expand_to_static_leaves(){
    let l=leaves("import p.net.{http.{self as h, fnc get as fetch, class Request}, tcp.{connect,},};");
    assert_eq!(l.iter().map(|x|(x.path.as_str(),x.binding.as_str(),x.kind)).collect::<Vec<_>>(),
        vec![("p.net.http","h",None),("p.net.http.get","fetch",Some(Kind::Fnc)),
        ("p.net.http.Request","Request",Some(Kind::Class)),("p.net.tcp.connect","connect",None)]);
    let (mut scope,view)=graph();
    bind(&mut scope,&l,&[&view]).unwrap();
    assert_eq!(scope.resolve_export("h.get",&view).unwrap().declaration,"net.http.get");
    assert_eq!(scope.resolve_export("fetch",&view).unwrap().declaration,"net.http.get");
    assert_eq!(scope.resolve_export("Request",&view).unwrap().kind,Kind::Class);
    assert_eq!(scope.resolve_export("connect",&view).unwrap().declaration,"net.tcp.connect");
}
#[test]
fn tree_selection_is_atomic_on_private_missing_or_wrong_kind(){
    for (src,code) in [
        ("import p.net.http.{get, secret};","E_PRIVATE"),
        ("import p.net.http.{get, missing};","E_UNBOUND"),
        ("import p.net.http.{get, class secret};","E_PRIVATE"),
        ("import p.net.http.{get, fnc Request};","E_IMPORT_KIND"),
    ]{
        let (mut scope,view)=graph();
        assert_eq!(bind(&mut scope,&leaves(src),&[&view]).unwrap_err().code,code);
        assert_eq!(scope.resolve("get").unwrap_err().code,"E_UNBOUND");
    }
}
#[test]
fn malformed_trees_fail_closed(){
    for src in [
        "import p.net.{};","import p.net.{get,,Request};","import p.net.{get,Request];",
        "import p.net.*;","import p.net.{get as f,Request as f};","import p.net.{self.child};",
        "import fnc p.net.{get};","import p.net.{get} as h;","import p.net.{self as a.b};",
        "import p.a.b.c.d.e.f.g.h;",
    ] {assert!(parse(&lex(src).unwrap(),1).is_err(),"{src}");}
}
#[test]
fn contextual_roots_are_namespace_paths_not_filesystem_traversal(){
    for (src,current,expected) in [
        ("pkg.network.http",None,"network.http"),
        ("self.utils",Some("network.http"),"network.http.utils"),
        ("super.config",Some("network.http"),"network.config"),
        ("super.super.common",Some("network.http"),"common"),
    ]{assert_eq!(contextual_path(src,current).unwrap(),expected);assert_eq!(leaves(&format!("import {src};"))[0].path,src);}
    for (src,current) in [
        ("self.utils",None),("super.config",None),("super.super.super.common",Some("network.http")),
        ("pkg.shared",None),("pkg..hidden",None),("pkg.__atto_hidden",None),("self.super.config",Some("network.http")),
    ]{assert!(contextual_path(src,current).is_err(),"{src}");}
}

#[test]
fn contextual_parent_cannot_erase_invalid_source_namespace() {
    for current in ["network.", "network.__atto_hidden", "network.super", "network.shared", "network.bad-name",
                    "a.b.c.d.e.f.g.h.i", "network..http"] {
        let e=contextual_path("super.config",Some(current)).unwrap_err();
        assert_eq!(e.code,"E_IMPORT_PATH","{current}");
    }
    for src in ["pkg", "self", "super", "super.super", "super.super.shared", "pkg.a.b.c.d.e.f.g.h"] {
        assert_eq!(contextual_path(src,Some("network.http")).unwrap_err().code,"E_IMPORT_PATH","{src}");
    }
    assert_eq!(contextual_path("super.super.common",Some("network.http")).unwrap(),"common");
    assert_eq!(contextual_path("pkg.a.b.c.d.e.f.g",None).unwrap(),"a.b.c.d.e.f.g");
}
#[test]
fn direct_tree_binding_api_checks_budget_before_resolution() {
    let (mut scope,view)=graph();
    assert_eq!(bind(&mut scope,&[],&[&view]).unwrap_err().code,"E_SYNTAX");
    let mut oversized=Vec::new();
    for i in 0..257 {
        oversized.push(atto_compiler::import_tree::Leaf {
            path:"p.net.http.get".into(),binding:format!("f{i}"),kind:Some(Kind::Fnc),
            explicit_alias:true,group_self:false
        });
    }
    assert_eq!(bind(&mut scope,&oversized,&[&view]).unwrap_err().code,"E_SOURCE_LIMIT");
    assert_eq!(scope.resolve("f0").unwrap_err().code,"E_UNBOUND");
}

#[test]
fn contextual_binding_uses_only_current_package_and_preserves_atomicity(){
    use atto_compiler::import_tree::bind_contextual;
    let (mut scope,view)=graph();
    bind_contextual(&mut scope,&leaves("import self.{get as fetch, Request};"),Some("net.http"),&view).unwrap();
    assert_eq!(scope.resolve_export("fetch",&view).unwrap().declaration,"net.http.get");
    bind_contextual(&mut scope,&leaves("import super.tcp as t;"),Some("net.http"),&view).unwrap();
    assert_eq!(scope.resolve_export("t.connect",&view).unwrap().declaration,"net.tcp.connect");
    let l=leaves("import pkg.net.http.{get as good, secret};");
    assert_eq!(bind_contextual(&mut scope,&l,None,&view).unwrap_err().code,"E_PRIVATE");
    assert_eq!(scope.resolve("good").unwrap_err().code,"E_UNBOUND");
    assert!(bind_contextual(&mut scope,&leaves("import super.super.super.net;"),Some("net.http"),&view).is_err());
}

#[test]
fn contextual_binding_checks_budget_before_resolution() {
    use atto_compiler::import_tree::bind_contextual;
    let (mut scope,view)=graph();
    assert_eq!(bind_contextual(&mut scope,&[],None,&view).unwrap_err().code,"E_SYNTAX");
    let leaf=leaves("import pkg.net.http.get as fetch;").remove(0);
    let oversized:Vec<_>=(0..257).map(|i| atto_compiler::import_tree::Leaf {path:leaf.path.clone(),binding:format!("f{i}"),kind:Some(Kind::Fnc),explicit_alias:true,group_self:false}).collect();
    assert_eq!(bind_contextual(&mut scope,&oversized,None,&view).unwrap_err().code,"E_SOURCE_LIMIT");
    assert_eq!(scope.resolve("f0").unwrap_err().code,"E_UNBOUND");
}

#[test]
fn parent_prefix_is_equivalent_to_super_and_cannot_escape() {
    for (path, expected) in [
        ("parent.config", "network.config"),
        ("parent.parent.common", "common"),
        ("parent.super.common", "common"),
        ("super.parent.common", "common"),
    ] {
        assert_eq!(contextual_path(path, Some("network.http")).unwrap(), expected);
        assert_eq!(leaves(&format!("import {path};"))[0].path, path);
    }
    for (path, current) in [
        ("parent.config", None),
        ("parent.parent.parent.config", Some("network.http")),
        ("self.parent.config", Some("network.http")),
        ("parent.super", Some("network.http")),
        ("parent.config", Some("network.parent")),
    ] {
        assert_eq!(contextual_path(path, current).unwrap_err().code, "E_IMPORT_PATH");
    }
    use atto_compiler::import_tree::bind_contextual;
    let (mut scope, view) = graph();
    bind_contextual(&mut scope, &leaves("import parent.tcp as t;"), Some("net.http"), &view).unwrap();
    assert_eq!(scope.resolve_export("t.connect", &view).unwrap().declaration, "net.tcp.connect");
    let denied = leaves("import parent.http.{get as fresh, secret};");
    assert_eq!(bind_contextual(&mut scope, &denied, Some("net.tcp"), &view).unwrap_err().code, "E_PRIVATE");
    assert_eq!(scope.resolve("fresh").unwrap_err().code, "E_UNBOUND");
}

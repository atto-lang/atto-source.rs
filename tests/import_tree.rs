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
        ("super.super.shared",Some("network.http"),"shared"),
    ]{assert_eq!(contextual_path(src,current).unwrap(),expected);assert_eq!(leaves(&format!("import {src};"))[0].path,src);}
    for (src,current) in [
        ("self.utils",None),("super.config",None),("super.super.super.shared",Some("network.http")),
        ("pkg..hidden",None),("pkg.__atto_hidden",None),("self.super.config",Some("network.http")),
    ]{assert!(contextual_path(src,current).is_err(),"{src}");}
}

#[test]
fn contextual_parent_cannot_erase_invalid_source_namespace() {
    for current in ["network.", "network.__atto_hidden", "network.super", "network.bad-name",
                    "a.b.c.d.e.f.g.h.i", "network..http"] {
        let e=contextual_path("super.config",Some(current)).unwrap_err();
        assert_eq!(e.code,"E_IMPORT_PATH","{current}");
    }
    for src in ["pkg", "self", "super", "super.super", "pkg.a.b.c.d.e.f.g.h"] {
        assert_eq!(contextual_path(src,Some("network.http")).unwrap_err().code,"E_IMPORT_PATH","{src}");
    }
    assert_eq!(contextual_path("super.super.shared",Some("network.http")).unwrap(),"shared");
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

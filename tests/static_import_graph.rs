//! Host-side static binding graph tests; not external-package guest evidence.
use atto_compiler::{import_v2::{parse_mixed, Kind}, lexer::lex,
    static_import_graph::{ExportView, Origin, PackageId, Scope, Target, Tier}};

fn id(name: &str) -> PackageId { PackageId::new(name, "1.0.0", &"a".repeat(64)).unwrap() }
fn builtin() -> Target {
    Target { origin: Origin::Builtin("std.http".into()), declaration: "http".into(), kind: Kind::Namespace }
}
fn external(name: &str) -> Origin { Origin::External(id(name)) }
fn graph() -> ExportView {
    let mut g = ExportView::new(external("@foo/bar/baz"));
    for (name, kind) in [("C", Kind::Class), ("D", Kind::Class), ("f", Kind::Fnc),
        ("g", Kind::Fnc), ("m", Kind::Module), ("n", Kind::Module), ("z", Kind::Namespace)] {
        g.add(name, name, kind, true).unwrap();
    }
    g.add("secret", "secret", Kind::Namespace, false).unwrap();
    g.add("secret.f", "secret.f", Kind::Fnc, true).unwrap();
    g.add("z.hidden", "z.hidden", Kind::Fnc, false).unwrap();
    g.add("z.open", "z.open", Kind::Fnc, true).unwrap();
    g.add("C.not_namespace", "C.not_namespace", Kind::Fnc, true).unwrap();
    g
}
fn mixed(src: &str) -> atto_compiler::import_v2::MixedImport {
    parse_mixed(&lex(src).unwrap(), 1).unwrap().0
}

#[test]
fn suffixes_preserve_authenticated_identity() {
    let p = id("@atto-lang/network/http");
    assert_eq!(p.suffixes(), ["http", "network.http", "atto_lang.network.http"]);
    let mut s = Scope::new();
    s.package(p.clone(), None).unwrap();
    for name in ["http.get", "network.http.get", "atto_lang.network.http.get"] {
        let r = s.resolve(name).unwrap();
        assert_eq!(r.target.origin, Origin::External(p.clone()));
        assert_eq!(r.remainder, "get");
    }
}
#[test]
fn same_project_collisions_are_lazy_and_order_independent() {
    for names in [["@atto-lang/network/http", "@acme/network/http"],
                  ["@acme/network/http", "@atto-lang/network/http"]] {
        let mut s = Scope::new();
        for name in names { s.package(id(name), None).unwrap(); }
        for name in ["http.get", "network.http.get", "http.nonexistent"] {
            assert_eq!(s.resolve(name).unwrap_err().code, "E_AMBIGUOUS");
        }
        assert_eq!(s.resolve("atto_lang.network.http.get").unwrap().target.origin, external("@atto-lang/network/http"));
        assert_eq!(s.resolve("acme.network.http.get").unwrap().target.origin, external("@acme/network/http"));
    }
}
#[test]
fn different_projects_disambiguate_intermediate_suffix() {
    let mut s = Scope::new();
    s.package(id("@a/network/http"), None).unwrap();
    s.package(id("@b/cloud/http"), None).unwrap();
    assert_eq!(s.resolve("http.get").unwrap_err().code, "E_AMBIGUOUS");
    assert_eq!(s.resolve("network.http.get").unwrap().target.origin, external("@a/network/http"));
    assert_eq!(s.resolve("cloud.http.get").unwrap().target.origin, external("@b/cloud/http"));
}
#[test]
fn normalization_and_version_do_not_merge_identities() {
    for other in [id("@atto_lang/network/http"),
        PackageId::new("@atto-lang/network/http", "2.0.0", &"b".repeat(64)).unwrap()] {
        let mut s = Scope::new();
        s.package(id("@atto-lang/network/http"), None).unwrap();
        s.package(other, None).unwrap();
        assert_eq!(s.resolve("atto_lang.network.http.get").unwrap_err().code, "E_AMBIGUOUS");
    }
}
#[test]
fn explicit_aliases_replace_suffixes_and_do_not_grant_builtin_identity() {
    let mut s = Scope::new();
    s.bind_verified("std.http", Tier::Implicit, builtin()).unwrap();
    s.package(id("@vendor/network/http"), Some("std.http")).unwrap();
    assert_eq!(s.resolve("std.http.get").unwrap().target.origin, external("@vendor/network/http"));
    assert_eq!(s.resolve("http.get").unwrap_err().code, "E_UNBOUND");
    assert_eq!(s.resolve("vendor.network.http.get").unwrap_err().code, "E_UNBOUND");
    s.bind_verified("official_http", Tier::Explicit, builtin()).unwrap();
    assert_eq!(s.resolve("official_http.get").unwrap().target.origin, builtin().origin);
    assert_eq!(s.package(id("@a/b/c"), Some("std.http")).unwrap_err().code, "E_DUPLICATE");
}
#[test]
fn tiers_do_not_fall_back_to_implicit_builtins() {
    let mut s = Scope::new();
    s.bind_verified("http", Tier::Implicit, builtin()).unwrap();
    s.bind_verified("std.http", Tier::Implicit, builtin()).unwrap();
    s.package(id("@a/network/http"), None).unwrap();
    assert_eq!(s.resolve("http.absent").unwrap().target.origin, external("@a/network/http"));
    assert_eq!(s.resolve("std.http.get").unwrap().target.origin, builtin().origin);
    s.bind_verified("http", Tier::Deliberate, builtin()).unwrap();
    assert_eq!(s.resolve("http.absent").unwrap_err().code, "E_AMBIGUOUS");
}
#[test]
fn explicit_non_namespace_blocks_lower_tiers_and_overlapping_prefix() {
    let mut s = Scope::new();
    s.bind_verified("http", Tier::Implicit, builtin()).unwrap();
    let f = Target { origin: Origin::Local("entry".into()), declaration: "http".into(), kind: Kind::Fnc };
    s.bind_verified("http", Tier::Explicit, f).unwrap();
    assert_eq!(s.resolve("http.get").unwrap_err().code, "E_IMPORT_KIND");
    assert_eq!(s.package(id("@a/b/c"), Some("http.child")).unwrap_err().code, "E_DUPLICATE");
}
#[test]
fn overlapping_namespace_prefixes_are_not_longest_match() {
    let mut s = Scope::new();
    s.package(id("@a/b/c"), Some("vendor")).unwrap();
    s.package(id("@d/e/f"), Some("vendor.child")).unwrap();
    assert_eq!(s.resolve("vendor.child.get").unwrap_err().code, "E_AMBIGUOUS");
}
#[test]
fn typed_selection_uses_parser_and_checks_export_identity() {
    let g = graph();
    let mut s = Scope::new();
    let m = mixed("import class [C, D as Data], fnc [f as fetch, g], module (m, n), namespace (z) from pkg '@foo/bar/baz';");
    s.bind_mixed(&g, &m).unwrap();
    for (local, remote, kind) in [("C","C",Kind::Class), ("Data","D",Kind::Class),
        ("fetch","f",Kind::Fnc), ("g","g",Kind::Fnc), ("m","m",Kind::Module),
        ("n","n",Kind::Module), ("z","z",Kind::Namespace)] {
        let t = s.resolve(local).unwrap().target;
        assert_eq!((t.declaration.as_str(), t.kind), (remote, kind));
        assert_eq!(t.origin, external("@foo/bar/baz"));
    }
    assert_eq!(s.resolve("D").unwrap_err().code, "E_UNBOUND");
}
#[test]
fn typed_list_failure_is_atomic_for_wrong_kind_private_missing_and_duplicate() {
    for (src, code) in [
        ("import class [C], fnc [D] from pkg '@foo/bar/baz';","E_IMPORT_KIND"),
        ("import class [C], fnc [secret.f] from pkg '@foo/bar/baz';","E_PRIVATE"),
        ("import class [C], fnc [z.hidden] from pkg '@foo/bar/baz';","E_PRIVATE"),
        ("import class [C], fnc [z.absent] from pkg '@foo/bar/baz';","E_UNBOUND"),
        ("import class [C], fnc [C.not_namespace] from pkg '@foo/bar/baz';","E_IMPORT_KIND"),
    ] {
        let mut s = Scope::new();
        assert_eq!(s.bind_mixed(&graph(), &mixed(src)).unwrap_err().code, code);
        assert_eq!(s.resolve("C").unwrap_err().code, "E_UNBOUND");
    }
    let mut s = Scope::new();
    s.bind_verified("fetch", Tier::Explicit, builtin()).unwrap();
    let m = mixed("import class [C], fnc [f as fetch] from pkg '@foo/bar/baz';");
    assert_eq!(s.bind_mixed(&graph(), &m).unwrap_err().code, "E_DUPLICATE");
    assert_eq!(s.resolve("C").unwrap_err().code, "E_UNBOUND");
}
#[test]
fn as_same_name_is_still_explicit_and_namespace_alias_is_not_value() {
    let mut s = Scope::new();
    s.package(id("@a/b/z"), None).unwrap();
    let m = mixed("import namespace [z as z] from pkg '@foo/bar/baz';");
    assert!(m.selectors[0].explicit_alias);
    s.bind_mixed(&graph(), &m).unwrap();
    let z = s.resolve("z").unwrap().target;
    assert_eq!(z.origin, external("@foo/bar/baz"));
    s.namespace_alias("alias", z.clone()).unwrap();
    assert_eq!(s.resolve("alias").unwrap().target, z);
    s.namespace_alias("chain", s.resolve("alias").unwrap().target).unwrap();
    assert_eq!(s.resolve("chain").unwrap().target, z);
    assert_eq!(s.namespace_alias("bad", graph().lookup("f").unwrap()).unwrap_err().code, "E_IMPORT_KIND");
}
#[test]
fn malformed_registry_and_binding_paths_fail_closed() {
    for name in ["@a/b","@a/b/c/d","@a//c","@a/../c","@a/b/c\\d","@a/b/c?x","https://example.test"] {
        assert_eq!(PackageId::new(name,"1.0.0",&"a".repeat(64)).unwrap_err().code,"E_IMPORT_PATH");
    }
    let mut s = Scope::new();
    for alias in ["", "a..b", "a/b", "class", "__atto_internal", "a.b.c.d.e.f.g.h.i"] {
        assert_eq!(s.package(id("@a/b/c"),Some(alias)).unwrap_err().code,"E_IMPORT_PATH");
    }
    assert!(s.resolve("c").is_err());
}
#[test]
fn capacity_is_atomic_across_automatic_suffixes() {
    let mut s = Scope::new();
    for i in 0..255 {
        s.package(id("@a/b/c"), Some(&format!("p{i}"))).unwrap();
    }
    assert_eq!(s.package(id("@a/b/c"), None).unwrap_err().code, "E_SOURCE_LIMIT");
    assert_eq!(s.resolve("c").unwrap_err().code, "E_UNBOUND");
    assert!(s.resolve("p254").is_ok());
}

#[test]
fn mismatched_export_view_cannot_supply_another_package_or_builtin() {
    let mut s = Scope::new();
    for src in [
        "import class [C] from pkg '@other/bar/baz';",
        "import class [C] from core.num;",
        "import class [C] from './lib.atto';",
        "import class [C] from pkg '@foo/bar/baz/subpath';",
    ] {
        assert_eq!(s.bind_mixed(&graph(), &mixed(src)).unwrap_err().code, "E_IMPORT_PATH");
        assert_eq!(s.resolve("C").unwrap_err().code, "E_UNBOUND");
    }
}

#[test]
fn missing_members_do_not_fall_back_and_interior_visibility_is_checked() {
    let mut s = Scope::new();
    s.package(id("@foo/bar/baz"), Some("p")).unwrap();
    let g = graph();
    assert_eq!(s.resolve_export("p.z.open", &g).unwrap().declaration, "z.open");
    assert_eq!(s.resolve_export("p.z.hidden", &g).unwrap_err().code, "E_PRIVATE");
    assert_eq!(s.resolve_export("p.z.absent", &g).unwrap_err().code, "E_UNBOUND");
    let other = ExportView::new(Origin::Builtin("std.http".into()));
    assert_eq!(s.resolve_export("p.z.open", &other).unwrap_err().code, "E_IMPORT_PATH");
}
#[test]
fn namespace_constant_forward_chains_and_interior_aliases_are_atomic() {
    let mut s = Scope::new();
    s.package(id("@foo/bar/baz"), Some("p")).unwrap();
    let g = graph();
    s.namespace_aliases(&[("first", "second"), ("second", "p.z")], &[&g]).unwrap();
    assert_eq!(s.resolve("first").unwrap().target.declaration, "z");
    assert_eq!(s.resolve_export("first.open", &g).unwrap().declaration, "z.open");
    assert_eq!(s.resolve_export("first.hidden", &g).unwrap_err().code, "E_PRIVATE");
    assert_eq!(s.namespace_aliases(&[("ok", "p"), ("bad", "p.z.hidden")], &[&g]).unwrap_err().code, "E_PRIVATE");
    assert_eq!(s.resolve("ok").unwrap_err().code, "E_UNBOUND");
    assert_eq!(s.namespace_aliases(&[("bad", "p.f")], &[&g]).unwrap_err().code, "E_IMPORT_KIND");
}
#[test]
fn pending_aliases_cannot_break_cycles_by_falling_back_to_deliberate_names() {
    let mut s = Scope::new();
    s.package(id("@a/network/http"), None).unwrap();
    assert_eq!(s.namespace_aliases(&[("http", "alias"), ("alias", "http")], &[]).unwrap_err().code, "E_IMPORT_CYCLE");
    assert_eq!(s.resolve("alias").unwrap_err().code, "E_UNBOUND");
    assert_eq!(s.resolve("http").unwrap().target.origin, external("@a/network/http"));
    assert_eq!(s.namespace_aliases(&[("x", "x")], &[]).unwrap_err().code, "E_IMPORT_CYCLE");
    assert_eq!(s.namespace_aliases(&[("x", "missing")], &[]).unwrap_err().code, "E_UNBOUND");
}

#[test]
fn all_alias_cycles_reject_before_any_export_lookup_and_publish_nothing() {
    for declarations in [
        vec![("z", "a"), ("a", "b"), ("b", "c"), ("c", "a"), ("missing", "unbound")],
        vec![("missing", "unbound"), ("c", "a"), ("b", "c"), ("a", "b"), ("z", "a")],
        vec![("a", "b.child"), ("b", "c"), ("c", "a")],
    ] {
        let mut scope = Scope::new();
        let d = scope.namespace_aliases(&declarations, &[]).unwrap_err();
        assert_eq!(d.code, "E_IMPORT_CYCLE");
        assert!(d.to_string().contains("a -> b -> c -> a"), "{d}");
        assert_eq!(scope.resolve("a").unwrap_err().code, "E_UNBOUND");
        assert_eq!(scope.resolve("z").unwrap_err().code, "E_UNBOUND");
    }
}

#[test]
fn long_alias_chains_are_bounded_and_nonrecursive() {
    let mut scope = Scope::new();
    scope.package(id("@a/b/c"), Some("root")).unwrap();
    let names: Vec<_> = (0..255).map(|i| format!("alias{i}")).collect();
    let aliases: Vec<_> = names.iter().enumerate().map(|(i,n)| {
        (n.as_str(), names.get(i+1).map_or("root", String::as_str))
    }).collect();
    scope.namespace_aliases(&aliases, &[]).unwrap();
    assert_eq!(scope.resolve("alias0").unwrap().target.origin, external("@a/b/c"));
    assert_eq!(scope.namespace_aliases(&[("extra", "root")], &[]).unwrap_err().code, "E_SOURCE_LIMIT");
    assert_eq!(scope.resolve("extra").unwrap_err().code, "E_UNBOUND");
}

#[test]
fn exact_binding_rechecks_kind_visibility_and_canonical_identity() {
    let g=graph();
    for (decl,kind,code) in [
        ("z.hidden",Kind::Fnc,"E_PRIVATE"),
        ("secret.f",Kind::Fnc,"E_PRIVATE"),
        ("C",Kind::Fnc,"E_IMPORT_KIND"),
        ("missing",Kind::Class,"E_UNBOUND"),
        ("",Kind::Class,"E_IMPORT_KIND"),
    ] {
        let mut s=Scope::new();
        s.bind_verified("forged",Tier::Explicit,Target{
            origin:external("@foo/bar/baz"),declaration:decl.into(),kind
        }).unwrap();
        assert_eq!(s.resolve_export("forged",&g).unwrap_err().code,code);
    }
    let mut s=Scope::new();
    s.bind_mixed(&g,&mixed("import class [C as Alias] from pkg '@foo/bar/baz';")).unwrap();
    assert_eq!(s.resolve_export("Alias",&g).unwrap(),g.lookup("C").unwrap());
    let mut changed=ExportView::new(external("@foo/bar/baz"));
    changed.add("C","C",Kind::Class,false).unwrap();
    assert_eq!(s.resolve_export("Alias",&changed).unwrap_err().code,"E_PRIVATE");
}

#[test]
fn nominal_identity_preserves_aliases_and_distinguishes_authenticated_locks() {
    let a=id("@foo/bar/baz");
    let b=PackageId::new("@foo/bar/baz","1.0.0",&"b".repeat(64)).unwrap();
    let c=PackageId::new("@other/bar/baz","1.0.0",&"a".repeat(64)).unwrap();
    let mut scope=Scope::new();
    let mut identities=Vec::new();
    for (alias,pkg) in [("a",a),("b",b),("c",c)] {
        let mut view=ExportView::new(Origin::External(pkg.clone()));
        for (name,kind) in [("n",Kind::Namespace),("n.Item",Kind::Class),
            ("n.State",Kind::Enum),("n.Worker",Kind::Actor),
            ("n.Alias",Kind::Type),("n.f",Kind::Fnc)] {
            view.add(name,name,kind,true).unwrap();
        }
        scope.package(pkg,Some(alias)).unwrap();
        let item=scope.resolve_nominal(&format!("{alias}.n.Item"),&view).unwrap();
        scope.bind_verified(&format!("{alias}_item"),Tier::Explicit,
            view.lookup("n.Item").unwrap()).unwrap();
        assert_eq!(item,scope.resolve_nominal(&format!("{alias}_item"),&view).unwrap());
        assert_ne!(item,scope.resolve_nominal(&format!("{alias}.n.State"),&view).unwrap());
        assert_ne!(item,scope.resolve_nominal(&format!("{alias}.n.Worker"),&view).unwrap());
        for name in ["n.Alias","n.f","n"] {
            assert_eq!(scope.resolve_nominal(&format!("{alias}.{name}"),&view).unwrap_err().code,"E_IMPORT_KIND");
        }
        identities.push(item);
    }
    assert_ne!(identities[0],identities[1]);
    assert_ne!(identities[0],identities[2]);
}


#[test]
fn nominal_identity_is_alias_independent_and_snapshot_bound() {
    let mut s=Scope::new();
    let p=id("@a/project/pkg");
    let q=id("@b/project/pkg");
    let changed=PackageId::new("@a/project/pkg","1.0.0",&"b".repeat(64)).unwrap();
    let mut views=Vec::new();
    for (package,alias) in [(p.clone(),"p"),(q,"q"),(changed,"changed")] {
        let mut v=ExportView::new(Origin::External(package.clone()));
        v.add("models","models",Kind::Namespace,true).unwrap();
        v.add("models.User","models.User",Kind::Class,true).unwrap();
        v.add("models.State","models.State",Kind::Enum,true).unwrap();
        v.add("models.helper","models.helper",Kind::Fnc,true).unwrap();
        v.add("models.Hidden","models.Hidden",Kind::Class,false).unwrap();
        s.package(package,Some(alias)).unwrap();
        views.push(v);
    }
    let refs:Vec<_>=views.iter().collect();
    let user=s.resolve_nominal_in_views("p.models.User",&refs).unwrap();
    assert_eq!(user.kind(),Kind::Class);
    assert_eq!(user.declaration(),"models.User");
    s.namespace_aliases(&[("m","p.models")],&refs).unwrap();
    assert_eq!(user,s.resolve_nominal_in_views("m.User",&refs).unwrap());
    let target=s.resolve_in_views("p.models.User",&refs).unwrap();
    s.bind_verified("Person",Tier::Explicit,target).unwrap();
    assert_eq!(user,s.resolve_nominal_in_views("Person",&refs).unwrap());
    assert_ne!(user,s.resolve_nominal_in_views("q.models.User",&refs).unwrap());
    assert_ne!(user,s.resolve_nominal_in_views("changed.models.User",&refs).unwrap());
    assert_ne!(user,s.resolve_nominal_in_views("p.models.State",&refs).unwrap());
    assert_eq!(s.resolve_nominal_in_views("p.models.helper",&refs).unwrap_err().code,"E_IMPORT_KIND");
    assert_eq!(s.resolve_nominal_in_views("p.models.Hidden",&refs).unwrap_err().code,"E_PRIVATE");
    assert_eq!(s.resolve_nominal_in_views("Person",&[&views[0],&views[0]]).unwrap_err().code,"E_AMBIGUOUS");
}

#[test]
fn direct_nominal_binding_cannot_forge_kind_or_visibility() {
    let mut s=Scope::new();
    let p=id("@a/project/pkg");
    let origin=Origin::External(p);
    let mut v=ExportView::new(origin.clone());
    v.add("models","models",Kind::Namespace,true).unwrap();
    v.add("models.User","models.User",Kind::Class,true).unwrap();
    v.add("models.Hidden","models.Hidden",Kind::Class,false).unwrap();
    for (alias,decl,kind,code) in [
        ("WrongKind","models.User",Kind::Enum,"E_IMPORT_KIND"),
        ("Private","models.Hidden",Kind::Class,"E_PRIVATE"),
        ("Missing","models.Missing",Kind::Class,"E_UNBOUND"),
    ] {
        s.bind_verified(alias,Tier::Explicit,Target{origin:origin.clone(),declaration:decl.into(),kind}).unwrap();
        assert_eq!(s.resolve_nominal_in_views(alias,&[&v]).unwrap_err().code,code);
    }
    let mut other=ExportView::new(Origin::External(id("@b/project/pkg")));
    other.add("models","models",Kind::Namespace,true).unwrap();
    other.add("models.User","models.User",Kind::Class,true).unwrap();
    assert_eq!(s.resolve_nominal_in_views("WrongKind",&[&other]).unwrap_err().code,"E_UNBOUND");
}


#[test]
fn generic_nominal_keys_preserve_aliases_argument_order_and_package_identity() {
    use atto_compiler::static_import_graph::{ScalarIdentity as S,TypeTerm as T};
    let mut scope=Scope::new();
    let mut views=Vec::new();
    for (pkg,alias) in [("@a/project/pkg","a"),("@b/project/pkg","b")] {
        let p=id(pkg);
        let mut v=ExportView::new(Origin::External(p.clone()));
        v.add("models","models",Kind::Namespace,true).unwrap();
        v.add_generic("models.Pair","models.Pair",Kind::Class,true,2).unwrap();
        v.add_generic("models.Wrap","models.Wrap",Kind::Type,true,1).unwrap();
        v.add("models.User","models.User",Kind::Class,true).unwrap();
        v.add("models.Hidden","models.Hidden",Kind::Class,false).unwrap();
        scope.package(p,Some(alias)).unwrap();
        views.push(v);
    }
    let refs:Vec<_>=views.iter().collect();
    scope.namespace_aliases(&[("m","a.models")],&refs).unwrap();
    let head=|reference|T::Nominal{reference,arguments:2};
    let k=scope.resolve_type_identity(&[T::Scalar(S::Int),T::Scalar(S::Bool),head("a.models.Pair")],&refs).unwrap();
    assert_eq!(k,scope.resolve_type_identity(&[T::Scalar(S::Int),T::Scalar(S::Bool),head("m.Pair")],&refs).unwrap());
    assert_ne!(k,scope.resolve_type_identity(&[T::Scalar(S::Bool),T::Scalar(S::Int),head("a.models.Pair")],&refs).unwrap());
    assert_ne!(k,scope.resolve_type_identity(&[T::Scalar(S::Int),T::Scalar(S::Bool),head("b.models.Pair")],&refs).unwrap());
    let user=|reference|T::Nominal{reference,arguments:0};
    let wrap=T::Nominal{reference:"a.models.Wrap",arguments:1};
    let u=scope.resolve_type_identity(&[user("a.models.User"),wrap.clone()],&refs).unwrap();
    assert_ne!(u,scope.resolve_type_identity(&[user("b.models.User"),wrap.clone()],&refs).unwrap());
    assert_eq!(scope.resolve_type_identity(&[user("a.models.Hidden"),wrap],&refs).unwrap_err().code,"E_PRIVATE");
}

#[test]
fn generic_nominal_keys_reject_arity_shape_and_resource_violations() {
    use atto_compiler::static_import_graph::{ScalarIdentity as S,TypeTerm as T};
    let p=id("@a/project/pkg");
    let mut v=ExportView::new(Origin::External(p.clone()));
    v.add_generic("Wrap","Wrap",Kind::Class,true,1).unwrap();
    v.add("Plain","Plain",Kind::Class,true).unwrap();
    assert_eq!(v.add_generic("Huge","Huge",Kind::Class,true,9).unwrap_err().code,"E_SOURCE_LIMIT");
    assert_eq!(v.add_generic("f","f",Kind::Fnc,true,1).unwrap_err().code,"E_IMPORT_KIND");
    let mut s=Scope::new();
    s.package(p,Some("p")).unwrap();
    let w=T::Nominal{reference:"p.Wrap",arguments:1};
    let scalar=T::Scalar(S::Int);
    assert_eq!(s.resolve_type_identity(&[w.clone()],&[&v]).unwrap_err().code,"E_ARITY");
    assert_eq!(s.resolve_type_identity(&[scalar.clone(),T::Nominal{reference:"p.Plain",arguments:1}],&[&v]).unwrap_err().code,"E_ARITY");
    assert_eq!(s.resolve_type_identity(&[scalar.clone(),scalar.clone()],&[&v]).unwrap_err().code,"E_ARITY");
    let mut deep=vec![scalar.clone()];
    deep.extend(std::iter::repeat(w).take(16));
    assert_eq!(s.resolve_type_identity(&deep,&[&v]).unwrap_err().code,"E_SOURCE_LIMIT");
    assert_eq!(s.resolve_type_identity(&vec![scalar;129],&[&v]).unwrap_err().code,"E_SOURCE_LIMIT");
}

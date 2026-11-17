//! Prelink policy tests for fully-qualified nominal class identities.
//! These authenticate a modeled graph, not executable cross-package linkage.
use atto_compiler::{
    lexer, nominal_link::{self, LinkClass, NominalId},
    nominal_lower::{self, ClassVisibility as V, Hierarchy as H},
};

fn id(package: &str, namespace: &str, name: &str) -> NominalId {
    NominalId { package:package.into(), namespace:namespace.into(), name:name.into() }
}
fn class(identity: NominalId, kind: H, visibility: V, parents: Vec<NominalId>, permits: Vec<NominalId>) -> LinkClass {
    LinkClass { id:identity, hierarchy:kind, visibility,
        pos:atto_compiler::Pos {line:1,col:1},
        direct_parents:parents, permitted_children:permits }
}
fn assert_code(graph: &[LinkClass], expected: &str) {
    let err = nominal_link::validate_prelink(graph)
        .expect_err("invalid fully-qualified class graph was authenticated");
    assert_eq!(err.code, expected, "{err}");
}
fn allowed() -> Vec<LinkClass> {
    let root=id("alpha","animals","Root");
    let dog=id("beta","pets","Dog");
    vec![
        class(root.clone(),H::Sealed,V::Public,vec![],vec![dog.clone()]),
        class(dog,H::Final,V::Public,vec![root],vec![]),
    ]
}
#[test]
fn explicitly_authorized_cross_namespace_parent_child_pass_policy_only() {
    nominal_link::validate_prelink(&allowed()).unwrap();
}
#[test]
fn same_short_class_name_from_foreign_package_cannot_impersonate_permitted_identity() {
    let mut graph=allowed();
    graph.push(class(id("gamma","pets","Dog"),H::Final,V::Public,
        vec![id("alpha","animals","Root")],vec![]));
    assert_code(&graph,"E_SEALED");
    let mut changed=allowed();
    changed[0].permitted_children=vec![id("gamma","pets","Dog")];
    assert_code(&changed,"E_UNBOUND");
}
#[test]
fn cross_package_or_namespace_private_parent_is_not_visible_even_when_permitted() {
    let mut graph=allowed();
    graph[0].visibility=V::Private;
    assert_code(&graph,"E_VISIBILITY");
    graph[0].visibility=V::Package;
    assert_code(&graph,"E_VISIBILITY");
    graph[1].id.package="alpha".into();
    graph[0].permitted_children=vec![graph[1].id.clone()];
    graph[1].direct_parents=vec![graph[0].id.clone()];
    nominal_link::validate_prelink(&graph).unwrap();
}
#[test]
fn two_sealed_parents_each_must_permit_the_exact_same_child_id() {
    let mut graph=allowed();
    let other=id("alpha","animals","Other");
    let dog=id("beta","pets","Dog");
    graph.insert(1,class(other.clone(),H::Sealed,V::Public,vec![],vec![dog.clone()]));
    graph[2].direct_parents.push(other.clone());
    nominal_link::validate_prelink(&graph).unwrap();
    graph[1].permitted_children=vec![id("gamma","pets","Dog")];
    assert_code(&graph,"E_UNBOUND");
}
#[test]
fn final_open_and_orphan_nonsealed_rules_fail_closed_at_prelink() {
    let mut graph=allowed();
    graph[0].hierarchy=H::Final;
    graph[0].permitted_children.clear();
    assert_code(&graph,"E_FINAL");
    graph=allowed();
    graph[1].hierarchy=H::Open;
    assert_code(&graph,"E_SEALED");
    graph=allowed();
    graph[1].hierarchy=H::NonSealed;
    graph[1].direct_parents.clear();
    assert_code(&graph,"E_SEALED");
}
#[test]
fn duplicate_ids_and_spoofed_qualified_segments_are_rejected() {
    let mut graph=allowed();
    graph.push(graph[1].clone());
    assert_code(&graph,"E_DUPLICATE");
    graph.pop();
    graph[1].id.namespace="../animals".into();
    assert_code(&graph,"E_NAMESPACE");
    graph=allowed();
    graph[1].direct_parents.push(graph[0].id.clone());
    assert_code(&graph,"E_DUPLICATE");
}
#[test]
fn cycles_across_namespaces_and_misleading_permits_do_not_get_authenticated() {
    let a=id("one","a","A");
    let b=id("two","b","B");
    let cyc=vec![
        class(a.clone(),H::Sealed,V::Public,vec![b.clone()],vec![b.clone()]),
        class(b.clone(),H::Sealed,V::Public,vec![a.clone()],vec![a.clone()]),
    ];
    assert_code(&cyc,"E_CYCLE");
    let wrong=vec![
        class(a.clone(),H::Sealed,V::Public,vec![],vec![b.clone()]),
        class(b,H::Final,V::Public,vec![],vec![]),
    ];
    assert_code(&wrong,"E_SEALED");
}
#[test]
fn local_compiler_manifests_are_scoped_to_distinct_packages_without_name_aliasing() {
    let source = "define sealed class Base permits Leaf as end define final class Leaf extends Base as end";
    let parsed=nominal_lower::lower_with_hierarchy(lexer::lex(source).unwrap()).unwrap();
    let a=nominal_link::qualify_local(&parsed.classes,"pkg_one","scope").unwrap();
    let b=nominal_link::qualify_local(&parsed.classes,"pkg_two","scope").unwrap();
    nominal_link::validate_prelink(&a).unwrap();
    nominal_link::validate_prelink(&b).unwrap();
    let mut combined=a;
    combined.extend(b);
    nominal_link::validate_prelink(&combined).unwrap();
    assert_ne!(combined[0].id,combined[2].id);
}

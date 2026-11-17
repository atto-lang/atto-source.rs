//! Checked nominal hierarchy metadata survives token method composition.
//! The emitted identity domain is explicitly *local*; the WebAssembly custom
//! section is never accepted as proof of a foreign package's type identity.
use atto_compiler::{compile, lexer, nominal_lower::{self, Hierarchy}};

fn take_leb(bytes: &[u8], cursor: &mut usize) -> usize {
    let mut value = 0usize;
    for shift in (0..35).step_by(7) {
        let b = *bytes.get(*cursor).expect("bounded LEB byte");
        *cursor += 1;
        value |= ((b & 0x7f) as usize) << shift;
        if b & 0x80 == 0 { return value; }
    }
    panic!("oversized LEB");
}
fn take_name(bytes: &[u8], cursor: &mut usize) -> String {
    let len = take_leb(bytes, cursor);
    let end = (*cursor).checked_add(len).expect("length overflow");
    let text = std::str::from_utf8(bytes.get(*cursor..end).expect("name bytes"))
        .expect("valid UTF-8").to_owned();
    *cursor = end;
    text
}
fn hierarchy_section(wasm: &[u8]) -> Option<Vec<u8>> {
    assert!(wasm.starts_with(b"\0asm\x01\0\0\0"));
    let mut cursor = 8usize;
    let mut found = None;
    while cursor < wasm.len() {
        let kind = wasm[cursor];
        cursor += 1;
        let len = take_leb(wasm, &mut cursor);
        let end = cursor.checked_add(len).expect("section size overflow");
        let body = wasm.get(cursor..end).expect("bounded section");
        if kind == 0 {
            let mut at = 0;
            let name = take_name(body, &mut at);
            if name == "atto.nominal.hierarchy.v1" {
                assert!(found.is_none(), "duplicate hierarchy section");
                found = Some(body[at..].to_vec());
            }
        }
        cursor = end;
    }
    found
}
#[derive(Debug, PartialEq, Eq)]
struct Entry {
    name: String,
    status: u8,
    public: u8,
    parents: Vec<String>,
    permits: Vec<String>,
}
fn decode(wasm: &[u8]) -> (u8, Vec<Entry>) {
    let payload = hierarchy_section(wasm).expect("hierarchy custom section");
    let mut cursor = 0;
    let domain = payload[cursor];
    cursor += 1;
    let count = take_leb(&payload, &mut cursor);
    assert!(count <= 96);
    let mut entries = Vec::new();
    for _ in 0..count {
        let name = take_name(&payload, &mut cursor);
        let status = payload[cursor]; cursor += 1;
        let public = payload[cursor]; cursor += 1;
        let parents_count = take_leb(&payload, &mut cursor);
        assert!(parents_count <= 8);
        let parents = (0..parents_count).map(|_| take_name(&payload, &mut cursor)).collect();
        let child_count = take_leb(&payload, &mut cursor);
        assert!(child_count <= 96);
        let permits = (0..child_count).map(|_| take_name(&payload, &mut cursor)).collect();
        entries.push(Entry { name, status, public, parents, permits });
    }
    assert_eq!(cursor, payload.len(), "no unverified section suffix");
    (domain, entries)
}

#[test]
fn validated_hierarchy_is_retained_in_typed_lowering_before_wasm() {
    let source = r#"
        export pub class Root;
        define sealed class Root permits Flexible, Leaf as end
        define final class Leaf extends Root as end
        define non-sealed class Flexible extends Root as end
        define class Open extends Flexible as end
        pub fnc main(): int { return 42; }
    "#;
    // The explicit export header must already have been lowered.
    let pre = atto_compiler::visibility_lower::lower(lexer::lex(source).unwrap()).unwrap();
    let result = nominal_lower::lower_with_hierarchy(pre.tokens).unwrap();
    assert_eq!(result.classes.len(), 4);
    let root = result.classes.iter().find(|c| c.name == "Root").unwrap();
    assert_eq!(root.hierarchy, Hierarchy::Sealed);
    assert!(root.visibility_public);
    assert_eq!(root.permitted_children, ["Flexible", "Leaf"]);
    assert!(root.direct_parents.is_empty());
    let leaf = result.classes.iter().find(|c| c.name == "Leaf").unwrap();
    assert_eq!(leaf.hierarchy, Hierarchy::Final);
    assert_eq!(leaf.direct_parents, ["Root"]);
}

#[test]
fn wasm_preserves_local_graph_metadata_without_claiming_link_authority() {
    let source = r#"
        export pub class Root;
        define sealed class Root permits Flexible, Leaf as end
        define final class Leaf extends Root as end
        define non-sealed class Flexible extends Root as end
        define class Open extends Flexible as end
        pub fnc main(): int { return 42; }
    "#;
    let wasm = compile(source).unwrap();
    assert_eq!(wasm, compile(source).unwrap());
    let (domain, classes) = decode(&wasm);
    assert_eq!(domain, 0, "not a namespace-bound or link-authenticated artifact");
    assert_eq!(classes, vec![
        Entry { name: "Flexible".into(), status: 2, public: 0, parents: vec!["Root".into()], permits: vec![] },
        Entry { name: "Leaf".into(), status: 3, public: 0, parents: vec!["Root".into()], permits: vec![] },
        Entry { name: "Open".into(), status: 0, public: 0, parents: vec!["Flexible".into()], permits: vec![] },
        Entry { name: "Root".into(), status: 1, public: 1, parents: vec![], permits: vec!["Flexible".into(), "Leaf".into()] },
    ]);
}

#[test]
fn rejected_or_unlinked_namespace_hierarchies_emit_no_executable_guest() {
    assert!(hierarchy_section(&compile("pub fnc main(): int {return 42;}").unwrap()).is_none());
    for source in [
        "define sealed class Root permits Missing as end pub fnc main(): int {return 1;}",
        "define sealed class Root permits Child as end define final class Child extends Root as end define class Evil extends Root as end pub fnc main(): int {return 1;}",
        "use namespace external; define sealed class Root permits Child as end define final class Child extends Root as end pub fnc main(): int {return 1;}",
        "define final class Root extends OtherModuleRoot as end pub fnc main(): int {return 1;}",
    ] {
        let d = compile(source).expect_err("unchecked namespace or subclass emitted guest");
        assert!(d.code.starts_with("E_"), "{d}");
        assert!(d.pos.line > 0 && d.pos.col > 0);
    }
}

#[test]
fn local_name_collision_does_not_infer_foreign_link_identity() {
    let first = compile("define sealed class Base permits Leaf as end define final class Leaf extends Base as end pub fnc main(): int {return 1;}").unwrap();
    let second = compile("define sealed class Base permits Leaf as end define final class Leaf extends Base as end pub fnc main(): int {return 2;}").unwrap();
    let (a, a_entries) = decode(&first);
    let (b, b_entries) = decode(&second);
    assert_eq!(a, 0);
    assert_eq!(b, 0);
    assert_eq!(a_entries, b_entries, "class labels alone are not globally unique");
}

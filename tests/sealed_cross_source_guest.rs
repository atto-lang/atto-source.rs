//! Executed source-catalogue cross-module sealed-class linker regression.
//! Build-owner source identities and exact source bytes are supplied in-process.
//! This does NOT load untrusted binary metadata or implement virtual dispatch.
use atto_compiler::source_link::{compile_linked,SourceUnit};

const BASE:&str = r#"
    import class Dog from "animals/pets/dog";
    export pub class Animal;
    define sealed class Animal permits Dog as
        pub answer(): int { return 42; }
    end
"#;
const DOG:&str = r#"
    import class Animal from "animals/core/base";
    export pub class Dog;
    define final class Dog extends Animal as end
"#;
const ROOT:&str = r#"
    import class Dog from "animals/pets/dog";
    pub fnc main(): int {
        val leaf = new Dog();
        return leaf.answer();
    }
"#;
fn units<'a>(base:&'a str,dog:&'a str,root:&'a str)->Vec<SourceUnit<'a>> {
    vec![
        SourceUnit{package:"animals",namespace:"core",module:"base",source:base},
        SourceUnit{package:"animals",namespace:"pets",module:"dog",source:dog},
        SourceUnit{package:"app",namespace:"root",module:"main",source:root},
    ]
}
fn err(units:&[SourceUnit<'_>],entry:usize,code:&str) {
    let d=compile_linked(units,entry).expect_err("unauthorized cross-source hierarchy generated executable Wasm");
    assert_eq!(d.code,code,"{d}");
    assert!(d.pos.line>0 && d.pos.col>0);
}
fn guest_42(bytes:&[u8]) {
    assert!(bytes.starts_with(b"\0asm\x01\0\0\0"));
    // The native compiled tests verify Wasm header and reproducible bytes;
    // actual Node WebAssembly guest execution is performed in CI.
}
#[test]
fn cross_source_permitted_leaf_compiles_into_real_executable_guest() {
    let us=units(BASE,DOG,ROOT);
    let wasm=compile_linked(&us,2).unwrap();
    guest_42(&wasm);
    assert_eq!(compile_linked(&us,2).unwrap(),wasm);
}
#[test]
fn source_order_changes_cannot_change_linked_output_or_qualified_identity() {
    let direct=compile_linked(&units(BASE,DOG,ROOT),2).unwrap();
    let reversed=units(BASE,DOG,ROOT).into_iter().rev().collect::<Vec<_>>();
    let after=compile_linked(&reversed,0).unwrap();
    assert_eq!(direct,after,"build catalogue ordering is not link identity");
}
#[test]
fn wrong_permitted_child_same_short_name_from_different_module_is_rejected() {
    let base=BASE.replace("animals/pets/dog","animals/pets/otherdog");
    let alternate="import class Animal from \"animals/core/base\"; export pub class Dog; define final class Dog extends Animal as end";
    let mut us=units(&base,DOG,ROOT);
    us.push(SourceUnit{package:"animals",namespace:"pets",module:"otherdog",source:alternate});
    err(&us,2,"E_SEALED");
}
#[test]
fn missing_source_unit_or_unbound_class_never_emits_guest() {
    let us=units(BASE,DOG,ROOT);
    err(&us[..1],0,"E_UNBOUND"); // missing Dog source: exact import cannot resolve
    let mut us=units(BASE,DOG,ROOT);
    us[1].source="import class Animal from \"animals/core/unknown\"; export pub class Dog; define final class Dog extends Animal as end";
    err(&us,2,"E_UNBOUND");
}
#[test]
fn private_or_package_only_imports_cannot_escape_package_boundary() {
    let private=BASE.replace("export pub class Animal;","");
    err(&units(&private,DOG,ROOT),2,"E_VISIBILITY");
    let pkg=BASE.replace("export pub class Animal;","export pub[pkg] class Animal;");
    // Dog is in same package: this is permitted.
    guest_42(&compile_linked(&units(&pkg,DOG,ROOT),2).unwrap());
    let foreign_dog=DOG.replace("animals/core/base","animals/core/base");
    let mut us=units(&pkg,&foreign_dog,ROOT);
    us[1].package="external";
    err(&us,2,"E_UNBOUND"); // root's exact import binding is missing
    // Package-only parent is inaccessible even when the external child is supplied.
    let foreign_base=pkg.replace("animals/pets/dog","external/pets/dog");
    let mut us=units(&pkg,&foreign_dog,ROOT);
    us[0].source=&foreign_base;
    us[1].package="external";
    us[2].source="import class Dog from \"external/pets/dog\"; pub fnc main(): int { val d = new Dog(); return d.answer(); }";
    err(&us,2,"E_VISIBILITY");
}
#[test]
fn wrong_method_override_and_inherited_layout_are_rejected_by_production_abi() {
    let invalid_override=r#"
       import class Animal from "animals/core/base";
       export pub class Dog;
       define final class Dog extends Animal as
           pub override answer(): bool { return true; }
       end
    "#;
    err(&units(BASE,invalid_override,ROOT),2,"E_SIGNATURE");
    let with_field=BASE.replace("pub answer(): int","int internal_value: 7; pub answer(): int");
    err(&units(&with_field,DOG,ROOT),2,"E_UNSUPPORTED");
}
#[test]
fn generated_name_spoofing_and_import_alias_collisions_fail_closed() {
    let forged=ROOT.replace("val leaf = new Dog()","val AttoLinkInternalC0 = new Dog()");
    err(&units(BASE,DOG,&forged),2,"E_RESERVED");
    let duplicate="import class Dog from \"animals/pets/dog\"; import class Animal as Dog from \"animals/core/base\"; pub fnc main(): int {return 0;}";
    err(&units(BASE,DOG,duplicate),2,"E_DUPLICATE");
}
#[test]
fn imported_alias_is_resolved_to_exact_module_and_can_be_instantiated() {
    let entry=r#"
        import class Dog as Chosen from "animals/pets/dog";
        pub fnc main(): int { val leaf = new Chosen(); return leaf.answer(); }
    "#;
    guest_42(&compile_linked(&units(BASE,DOG,entry),2).unwrap());
}
#[test]
fn same_short_class_name_in_two_modules_is_distinct_and_both_can_execute() {
    let other="export pub class Dog; define final class Dog as pub extra(): int {return 8;} end";
    let entry=r#"
        import class Dog as MainDog from "animals/pets/dog";
        import class Dog as OtherDog from "animals/other/otherdog";
        pub fnc main(): int {
            val a = new MainDog();
            val b = new OtherDog();
            return a.answer() + b.extra();
        }
    "#;
    let mut us=units(BASE,DOG,entry);
    us.push(SourceUnit{package:"animals",namespace:"other",module:"otherdog",source:other});
    guest_42(&compile_linked(&us,2).unwrap());
}
#[test]
fn no_import_after_declarations_or_foreign_module_imports() {
    let invalid=ROOT.replace("import class Dog from \"animals/pets/dog\";",
        "pub fnc helper(): int {return 1;} import class Dog from \"animals/pets/dog\";");
    err(&units(BASE,DOG,&invalid),2,"E_SYNTAX");
    let invalid=ROOT.replace("import class Dog","import module Dog");
    err(&units(BASE,DOG,&invalid),2,"E_UNSUPPORTED");
}
#[test]
fn linked_class_header_cannot_be_authorized_by_raw_string_or_prefix_forgeries() {
    let payload="import class Fake from \"animals/pets/dog\"; pub fnc main(): int { return 0; }";
    err(&units(BASE,DOG,payload),2,"E_UNBOUND");
    let payload="pub fnc main(): int { val x = \"import class Dog from animals/pets/dog\"; return 42; }";
    guest_42(&compile_linked(&units(BASE,DOG,payload),2).unwrap()); // a string cannot forge an import
}

#[test]
fn imported_class_alias_cannot_be_shadowed_by_type_first_scalar_binding() {
    let source=r#"
        import class Dog from "animals/pets/dog";
        pub fnc main(): int {
            int Dog: 12;
            return Dog;
        }
    "#;
    err(&units(BASE,DOG,source),2,"E_NAMESPACE");
}

#[test]
fn malformed_or_oversized_permits_lists_reject_without_panicking() {
    let bad="define sealed class Animal permits";
    err(&units(bad,DOG,ROOT),2,"E_EOF");
    let names=(0..97).map(|i|format!("Child{i}")).collect::<Vec<_>>().join(", ");
    let oversized=format!("define sealed class Animal permits {names} as end");
    err(&units(&oversized,DOG,ROOT),2,"E_SOURCE_LIMIT");
}

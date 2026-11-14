use atto_compiler::ast::Program;
use atto_compiler::lexer::lex;
use atto_compiler::target_syntax::parse_target_declarations;
use atto_compiler::target_typed::{build_actor_program, lower_target_signatures};
use atto_compiler::check::check_target_actor_interface;
use atto_compiler::Pos;

fn link(source: &str) -> Result<(atto_compiler::target_typed::TypedActorModule, Program), atto_compiler::Diagnostic> {
    let tokens=lex(source)?;
    let headers=parse_target_declarations(&tokens)?;
    let typed=lower_target_signatures(&headers)?;
    let program=build_actor_program(&headers, &typed);
    check_target_actor_interface(&program)?;
    Ok((typed,program))
}
fn reject(source: &str, code: &str) {
    let d=link(source).err().expect("expected link failure");
    assert_eq!(d.code, code, "link: {d}");
    let d=atto_compiler::compile(source).err().expect("expected compile rejection");
    assert_eq!(d.code,code, "compile: {d}");
}

const LINKED: &str = r#"
use namespace workers;
declare actor Foo1;
declare actor Foo2;
define actor Foo1 as
    receive(ActorMail<int> mail): void { return; }
end
define actor Foo2 as
    receive(ActorMail<int> mail): void { return; }
end
mtd (Foo1 self) foo(): int { return 10; }
mtd (Foo2 self) foo(): int { return 20; }
"#;

#[test]
fn link_two_concrete_actors_to_distinct_forward_declarations() {
    let (typed, program)=link(LINKED).unwrap();
    assert_eq!(typed.namespace.as_deref(),Some("workers"));
    assert_eq!(typed.actors.len(),2);
    assert!(typed.actors.iter().all(|a| a.has_definition));
    assert!(typed.actors.iter().all(|a| !a.defined));
    assert_eq!(program.actors.len(),2);
    assert!(program.actors.iter().all(|a| a.receive.is_some()));
    let iface=program.actor_interface.as_ref().unwrap();
    assert_eq!(iface.forwards,vec!["Foo1","Foo2"]);
    assert_eq!(iface.methods.len(),2);
    assert_eq!(iface.methods[0].owner,0);
    assert_eq!(iface.methods[1].owner,1);
    assert_eq!(iface.methods[0].ret,atto_compiler::ast::Type::Int);
    assert_eq!(typed.require_spawnable("Foo1",Pos{line:1,col:1}).unwrap_err().code,"E_UNLINKED");
    assert_eq!(atto_compiler::compile(LINKED).unwrap_err().code,"E_UNSUPPORTED");
}

#[test]
fn actor_definitions_are_real_production_ast_nodes() {
    let s=r#"
use namespace workers;
declare actor Counter;
define actor Counter as
    let int count = 0;
    receive(ActorMail<int> mail): void {
        self.count = self.count + mail.value;
        return;
    }
end
mtd (Counter self) get_count(): int { return 5; }
"#;
    let (_,p)=link(s).unwrap();
    assert_eq!(p.actors.len(),1);
    assert_eq!(p.actors[0].fields.len(),1);
    assert_eq!(p.actors[0].fields[0].name,"count");
    assert!(p.actors[0].receive.is_some());
    assert_eq!(p.actor_interface.as_ref().unwrap().methods[0].name,"get_count");
    assert_eq!(atto_compiler::compile(s).unwrap_err().code,"E_UNSUPPORTED");
}

#[test]
fn forward_only_actor_remains_unlinked_and_unspawnable() {
    let (typed,program)=link("use namespace workers;
        declare actor Foo;
        mtd (Foo self) ping(): int { return 1; }").unwrap();
    assert!(!typed.actors[0].has_definition);
    assert!(!typed.actors[0].defined);
    assert!(program.actors.is_empty());
    assert_eq!(typed.require_spawnable("Foo",Pos{line:1,col:1}).unwrap_err().code,"E_UNLINKED");
}

#[test]
fn unknown_and_duplicate_implementations_fail_closed() {
    reject("use namespace workers;
        declare actor Foo;
        define actor Bar as end","E_UNBOUND");
    reject("use namespace workers;
        declare actor Foo;
        define actor Foo as end
        define actor Foo as end","E_DUPLICATE");
    reject("use namespace workers;
        define actor Foo as end","E_UNBOUND");
}
#[test]
fn implementation_method_signature_collisions_are_rejected() {
    reject("use namespace workers;
        declare actor Foo;
        define actor Foo as
           pub foo(): int {return 1;}
        end
        pub mtd (Foo self) foo(): int {return 2;}","E_DUPLICATE");
    reject("use namespace workers;
        declare actor Foo;
        define actor Foo as
           foo(): int {return 1;}
        end
        pub mtd (Foo self) foo(): bool {return true;}","E_SIGNATURE");
}
#[test]
fn concrete_fields_cannot_collide_with_extension_methods() {
    reject("use namespace workers;
        declare actor Foo;
        define actor Foo as let int count = 0; end
        mtd (Foo self) count(): int {return 0;}","E_DUPLICATE");
}
#[test]
fn production_parser_rejects_malformed_concrete_actor_bodies() {
    reject("use namespace workers;
        declare actor Foo;
        define actor Foo as
            receive(ActorMail<int> mail): int {return 0;}
        end","E_TYPE");
    let err=link("use namespace workers;
        declare actor Foo;
        define actor Foo as
            receive(ActorMail<int> mail): void { return; }")
        .err().expect("unclosed actor must fail");
    assert_eq!(err.code,"E_EOF");
}
#[test]
fn regular_concrete_actor_programs_remain_on_executable_path() {
    let s="define actor W as
        receive(ActorMail<int> mail): void {return;}
        end
        pub fnc main(): int { val w=spawn W(); return 0; }";
    let bytes=atto_compiler::compile(s).expect("ordinary actor form still supported");
    assert_eq!(&bytes[..4],b"\0asm");
}

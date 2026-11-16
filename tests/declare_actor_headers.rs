//! Historical namespace actor declaration compatibility + typed pipe headers.
//!
//! A nominal declaration has no body, no implicit receiver, and cannot be
//! emitted as an executable guest actor before trusted package linking.
use atto_compiler::ast::{ActorDecl, Program, Type};
use atto_compiler::{lexer, parser};

fn parse(source: &str) -> Program {
    parser::Parser::new(lexer::lex(source).unwrap())
        .parse_program().unwrap()
}
fn actor(source: &str) -> (Program, ActorDecl) {
    let p = parse(source);
    assert_eq!(p.actors.len(), 1);
    let a = p.actors[0].clone();
    (p, a)
}
fn syntax_error(source: &str) -> String {
    let tokens = lexer::lex(source).unwrap();
    parser::Parser::new(tokens).parse_program().unwrap_err().code.to_string()
}
fn compile_error(source: &str) -> atto_compiler::diag::Diagnostic {
    atto_compiler::compile(source).unwrap_err()
}

#[test]
fn historical_semicolon_actor_header_is_private_and_nominal_only() {
    let (_, a) = actor("declare actor Foo;");
    assert_eq!(a.name, "Foo");
    assert!(a.declaration_only);
    assert!(!a.public);
    assert!(a.fields.is_empty());
    assert!(a.methods.is_empty());
    assert!(a.receive.is_none());
    assert_eq!(a.declared_receives, None);
    assert_eq!(a.declared_emits, None);
    assert_eq!(a.declared_sends, None);
}

#[test]
fn declared_x_y_roles_remain_unresolved_metadata_until_package_linking() {
    let (p, a) = actor("declare actor Foo receives x emits y;");
    assert!(a.declaration_only);
    let Type::Named(input) = a.declared_receives.unwrap() else { panic!("expected x"); };
    let Type::Named(output) = a.declared_emits.unwrap() else { panic!("expected y"); };
    assert_eq!(p.type_names[input as usize].0, "x");
    assert_eq!(p.type_names[output as usize].0, "y");
    assert_eq!(a.declared_sends, None);
}

#[test]
fn declared_public_actor_is_only_canonical_after_declare() {
    let (_, a) = actor("declare pub actor Foo receives int emits bool;");
    assert!(a.public);
    assert!(a.declaration_only);
    assert_eq!(a.declared_receives, Some(Type::Int));
    assert_eq!(a.declared_emits, Some(Type::Bool));
    assert_eq!(syntax_error("pub declare actor Foo;"), "E_ACTOR");
    assert!(syntax_error("private declare actor Foo;").len() > 0);
}

#[test]
fn explicit_private_actor_and_contract_references_preserve_metadata() {
    let (_, a) = actor("declare private actor Foo implements ActorXYZ, Auditable receives int emits bool;");
    assert!(!a.public);
    assert_eq!(a.implemented_contracts, vec!["ActorXYZ", "Auditable"]);
    assert!(a.declaration_only);
}

#[test]
fn sends_and_emits_are_independent_declaration_capabilities() {
    let (_, a) = actor("declare actor Foo receives int emits bool sends int;");
    assert_eq!(a.declared_receives, Some(Type::Int));
    assert_eq!(a.declared_emits, Some(Type::Bool));
    assert_eq!(a.declared_sends, Some(Type::Int));
    let (_, b) = actor("declare actor Foo receives int sends int emits bool;");
    assert_eq!(b.declared_receives, a.declared_receives);
    assert_eq!(b.declared_sends, a.declared_sends);
    assert_eq!(b.declared_emits, a.declared_emits);
}

#[test]
fn sends_only_is_not_an_implicit_pipe_source() {
    let (_, a) = actor("declare actor Foo sends int;");
    assert_eq!(a.declared_sends, Some(Type::Int));
    assert_eq!(a.declared_emits, None);
}

#[test]
fn inline_definitions_keep_their_original_implementation_shape() {
    let (_, a) = actor(
        "define actor Foo receives int emits bool as\n\
           receive(ActorMail<int> mail): void { emit(true); }\n\
         end"
    );
    assert!(!a.declaration_only);
    assert_eq!(a.declared_receives, Some(Type::Int));
    assert_eq!(a.declared_emits, Some(Type::Bool));
    assert!(a.receive.is_some());
}

#[test]
fn inline_definition_with_sends_parses_but_cannot_fake_transitive_effect_proof() {
    let source = "define actor Foo receives int emits int sends bool as\n\
                  receive(ActorMail<int> mail): void { emit(mail.value); }\n\
                  end\n\
                  pub fnc main(): int { return 0; }";
    assert_eq!(parse(source).actors[0].declared_sends, Some(Type::Bool));
    let error = compile_error(source);
    assert_eq!(error.code, "E_UNSUPPORTED");
    assert!(error.message.contains("transitive application-message effects"), "{error}");
}

#[test]
fn declaration_without_body_does_not_invent_runtime_receive_or_constructor() {
    let source = "declare actor Foo receives int emits bool;\n\
                  pub fnc main(): int { return 0; }";
    let err = compile_error(source);
    assert_eq!(err.code, "E_UNSUPPORTED");
    assert!(err.message.contains("metadata only"), "{err}");
    assert!(err.message.contains("external init/receive"), "{err}");
}

#[test]
fn declaration_cannot_be_spawned_without_authenticated_external_implementation() {
    let source = "declare actor Foo receives int emits int;\n\
                  pub fnc main(): int { val worker = spawn Foo(); return 0; }";
    let err = compile_error(source);
    assert_eq!(err.code, "E_UNSUPPORTED");
    assert!(err.message.contains("metadata only"), "{err}");
}

#[test]
fn define_actor_semicolon_cannot_masquerade_as_nominal_declaration() {
    assert_eq!(syntax_error("define actor Foo;"), "E_ACTOR");
}

#[test]
fn declarations_require_semicolon_not_inline_body() {
    for source in [
        "declare actor Foo as end",
        "declare actor Foo receives int as end",
        "declare actor Foo {}",
    ] {
        assert!(!syntax_error(source).is_empty(), "{source}");
    }
}

#[test]
fn duplicate_roles_and_receives_after_outputs_are_rejected() {
    for source in [
        "declare actor Foo receives int receives bool;",
        "declare actor Foo emits int emits int;",
        "declare actor Foo sends int sends bool;",
        "declare actor Foo emits int receives int;",
        "declare actor Foo sends int receives int;",
    ] {
        assert_eq!(syntax_error(source), "E_PIPE_CONTRACT", "{source}");
    }
}

#[test]
fn duplicate_contract_reference_never_counts_as_a_witness() {
    assert_eq!(
        syntax_error("declare actor Foo implements ActorXYZ, ActorXYZ;"),
        "E_DUPLICATE"
    );
    let source = "declare actor Foo implements ActorXYZ;\n\
                  pub fnc main(): int { return 0; }";
    let err = compile_error(source);
    assert_eq!(err.code, "E_UNSUPPORTED");
}

#[test]
fn unverified_actor_inheritance_and_generics_fail_closed() {
    for source in [
        "declare actor Foo extends Bar;",
        "declare actor Foo<int>;",
    ] {
        assert!(!syntax_error(source).is_empty(), "{source}");
    }
}

#[test]
fn repeated_nominal_actor_declarations_are_not_silently_merged() {
    let source = "declare actor Foo;\n\
                  declare actor Foo receives int;\n\
                  pub fnc main(): int { return 0; }";
    assert_eq!(compile_error(source).code, "E_DUPLICATE");
}

#[test]
fn inline_and_external_actor_forms_cannot_clash_without_package_linker() {
    let source = "declare actor Foo receives int;\n\
                  define actor Foo receives int as\n\
                    receive(ActorMail<int> mail): void { return; }\n\
                  end\n\
                  pub fnc main(): int { return 0; }";
    assert_eq!(compile_error(source).code, "E_DUPLICATE");
}

#[test]
fn bare_declaration_and_inline_definition_can_coexist_for_distinct_names() {
    let p = parse(
        "declare actor External receives int;\n\
         define actor Local as\n\
             receive(ActorMail<int> mail): void { return; }\n\
         end"
    );
    assert_eq!(p.actors.len(), 2);
    assert!(p.actors[0].declaration_only);
    assert!(!p.actors[1].declaration_only);
}

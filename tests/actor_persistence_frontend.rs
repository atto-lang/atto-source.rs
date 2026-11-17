//! Frontend and scalar codec evidence only: NOT durable process recovery.
use atto_compiler::{
    actor_persistence::{ActorStateSchema, StateScalar},
    ast::{ActorPersistenceMode, FieldMode},
    compile, lexer, parser::Parser,
};

fn parse(s: &str) -> Result<atto_compiler::ast::Program, atto_compiler::Diagnostic> {
    Parser::new(lexer::lex(s).unwrap()).parse_program()
}

#[test]
fn new_actor_modifiers_survive_parsing_and_volatiles_remain_distinct() {
    let p = parse("define durable actor A as state count: int = 0; end\n\
                   define persistent actor B as state active: bool = false; end\n\
                   define actor C as let int count = 0; end\n\
                   pub fnc main(): int { return 0; }").unwrap();
    assert_eq!(p.actors.len(), 3);
    assert_eq!(p.actors[0].persistence, ActorPersistenceMode::Durable);
    assert_eq!(p.actors[1].persistence, ActorPersistenceMode::Persistent);
    assert_eq!(p.actors[2].persistence, ActorPersistenceMode::Volatile);
    assert_eq!(p.actors[0].fields[0].mode, FieldMode::ActorState);
}

#[test]
fn canonical_pub_and_legacy_visibility_are_understood() {
    for src in [
        "define pub durable actor A as end",
        "pub define durable actor A as end",
        "define pub persistent actor A as end",
    ] {
        let p = parse(src).unwrap();
        assert_eq!(p.actors.len(), 1);
        assert_ne!(p.actors[0].persistence, ActorPersistenceMode::Volatile);
    }
}

#[test]
fn modifier_collisions_and_wrong_kinds_are_diagnostics() {
    for src in [
        "define durable persistent actor A as end",
        "define persistent durable actor A as end",
        "define durable class A as end",
        "define persistent struct A as end",
    ] {
        assert_eq!(parse(src).unwrap_err().code, "E_ACTOR_PERSISTENCE_MODIFIER");
    }
}

#[test]
fn no_automatic_disk_artifact_without_implemented_runtime() {
    let durable = "define durable actor A as state count: int = 0; end\n\
                   pub fnc main(): int { return 0; }";
    assert_eq!(compile(durable).unwrap_err().code, "E_DURABLE_RUNTIME_UNAVAILABLE");
    let persistent = "define persistent actor A as state count: int = 0; end\n\
                      pub fnc main(): int { return 0; }";
    assert_eq!(compile(persistent).unwrap_err().code, "E_PERSIST_HOOKS");
}

#[test]
fn custom_durable_hooks_are_not_silently_granted_authority() {
    let src = "define durable actor A as\n\
                 save_state(): void { return; }\n\
                 end\n pub fnc main(): int { return 0; }";
    assert_eq!(compile(src).unwrap_err().code, "E_DURABLE_HOOK_OVERRIDE");
}

#[test]
fn nonpersistable_type_fails_before_disk_emission() {
    let src = "define durable actor A as state socket: Socket = 0; end\n\
                 pub fnc main(): int { return 0; }";
    let p = parse(src).unwrap();
    assert_eq!(ActorStateSchema::for_actor(&p.actors[0]).unwrap_err().code,
        "E_NON_PERSISTABLE_STATE");
    assert_eq!(compile(src).unwrap_err().code, "E_NON_PERSISTABLE_STATE");
}

#[test]
fn compiled_scalar_schema_is_stable_and_roundtrips_owned_values() {
    let src = "define durable actor Counter as\n\
                 state count: int = 0;\n\
                 state enabled: bool = false;\n end";
    let p = parse(src).unwrap();
    let schema = ActorStateSchema::for_actor(&p.actors[0]).unwrap();
    let values = [StateScalar::Int(i64::MIN), StateScalar::Bool(true)];
    let bytes = schema.encode(&values).unwrap();
    assert_eq!(schema.decode(&bytes).unwrap(), values.to_vec());
    assert_eq!(ActorStateSchema::for_actor(&p.actors[0]).unwrap().schema_id,
        schema.schema_id);
    let mismatch = schema.encode(&[StateScalar::Bool(true), StateScalar::Int(7)]);
    assert_eq!(mismatch.unwrap_err().code, "E_PERSISTENCE_IO");
    let other = parse("define durable actor Counter as\n\
        state count: int = 0;\n state renamed: bool = false;\n end").unwrap();
    assert_eq!(ActorStateSchema::for_actor(&other.actors[0])
        .unwrap().decode(&bytes).unwrap_err().code, "E_PERSIST_SCHEMA_MISMATCH");
}

#[test]
fn codec_rejects_torn_corrupt_and_noncanonical_snapshots() {
    let p = parse("define durable actor Counter as state active: bool = true; end").unwrap();
    let schema = ActorStateSchema::for_actor(&p.actors[0]).unwrap();
    let valid = schema.encode(&[StateScalar::Bool(true)]).unwrap();
    for cut in 0..valid.len() {
        assert!(schema.decode(&valid[..cut]).is_err(), "torn cut {cut} accepted");
    }
    let mut corrupt = valid.clone();
    corrupt[19 + 1] = 2; // invalid bool payload
    assert_eq!(schema.decode(&corrupt).unwrap_err().code, "E_PERSISTENCE_IO");
    let mut corrupt = valid.clone();
    corrupt[19] = 1; // wrong field tag
    assert_eq!(schema.decode(&corrupt).unwrap_err().code, "E_PERSISTENCE_IO");
    let mut corrupt = valid;
    let len = corrupt.len();
    corrupt[len - 1] ^= 0x40;
    assert_eq!(schema.decode(&corrupt).unwrap_err().code, "E_PERSISTENCE_IO");
}

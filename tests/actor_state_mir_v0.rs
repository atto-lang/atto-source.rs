//! Real source -> production actor parser -> bounded typed-field events.
//! These tests do NOT certify actor guest code or a full CFG/NLL checker.
use atto_compiler::actor_state_mir_v0::{derive_bounded_guard_mir, FieldEventKind};
use atto_compiler::compile;

fn valid() -> &'static str {
    r#"declare actor Meter;
define actor Meter as
    let int counter = 0;
    let int xyz = 0;
end
mtd (Meter mut this.state.counter) update(): void {
    this.handle_state(critical-write |s| -> {
        s.counter += 1;
        s.xyz;
    });
}
"#
}
#[test]
fn source_bound_guard_events_are_exact_order_and_fields() {
    let mir = derive_bounded_guard_mir(valid()).unwrap();
    assert_eq!(mir.actor, "Meter");
    assert_eq!(mir.method, "update");
    assert_eq!(mir.field_count, 2);
    assert!(mir.exclusive);
    assert_eq!(mir.events.len(), 3);
    assert_eq!(mir.events.iter().map(|e| (e.kind, e.field)).collect::<Vec<_>>(),
        [(FieldEventKind::Read, 0), (FieldEventKind::Write, 0), (FieldEventKind::Read, 1)]);
    let wire = mir.to_wire();
    assert!(wire.starts_with("ATTO_ACTOR_STATE_MIR_V0\t"));
    assert!(wire.contains("\nROOT\t1\t2\nLOAN\t1\tEXCLUSIVE\n"));
    assert!(wire.contains("\nREAD\t0\t"));
    assert!(wire.contains("\nWRITE\t0\t"));
    assert!(wire.ends_with("\nEND\t1\nDONE\n"));
    assert_eq!(compile(valid()).unwrap_err().code, "E_UNSUPPORTED",
        "no guest can be emitted from this bounded source-MIR oracle");
}

fn rejected(body: &str, expected: &str) {
    let src = format!(r#"declare actor Meter;
define actor Meter as let int counter = 0; end
mtd (Meter mut this.state.counter) update(): void {{
    this.handle_state(critical-write |s| -> {{ {body} }});
}}"#);
    let e = derive_bounded_guard_mir(&src).expect_err("source effect must fail closed");
    assert_eq!(e.code, expected, "{e} -- source: {body}");
}
#[test]
fn calls_aliases_loops_and_nonliteral_writes_never_become_complete_effects() {
    for s in [
        "helper();", "s.counter = helper();", "if true then s.counter = 3; fi",
        "return s.counter;", "s.counter[0] = 1;", "s.counter.other = 1;",
        "s.counter = 1 + 2;", "s.counter = -1;", "s.counter = s.counter;",
        "await other;", "let int x = 1;", "s.counter++;",
    ] {
        let e = derive_bounded_guard_mir(&format!(r#"declare actor Meter;
define actor Meter as let int counter = 0; end
mtd (Meter mut this.state.counter) update(): void {{
 this.handle_state(critical-write |s| -> {{ {s} }});
}}"#));
        assert!(e.is_err(), "must reject incompletely modeled input: {s}");
    }
    rejected("s.counter = helper();", "E_MIR_UNSUPPORTED_EFFECT");
}
#[test]
fn concrete_unknown_fields_and_permissions_fail_before_mir_output() {
    rejected("s.missing;", "E_STATE_FIELD_UNKNOWN");
    rejected("s.counter += 9223372036854775808;", "E_MIR_UNSUPPORTED_EFFECT");
    let src = valid().replace("this.state.counter", "this.state.xyz");
    assert_eq!(derive_bounded_guard_mir(&src).unwrap_err().code, "E_MUTABILITY");
}
#[test]
fn readonly_scalar_guard_generates_readonly_loan() {
    let src = r#"declare actor Meter;
define actor Meter as let int counter = 0; end
mtd (Meter this) inspect(): void {
 this.handle_state(critical-read |s| -> { s.counter; });
}
"#;
    let mir = derive_bounded_guard_mir(src).unwrap();
    assert!(!mir.exclusive);
    assert_eq!(mir.events.len(), 1);
    assert_eq!(mir.events[0].kind, FieldEventKind::Read);
    assert!(mir.to_wire().contains("\nLOAN\t1\tSHARED\n"));
}
#[test]
fn trailing_receiver_statements_are_not_silently_omitted() {
    let src = valid().replace("    });\n}", "    });\n    helper();\n}");
    assert_eq!(derive_bounded_guard_mir(&src).unwrap_err().code, "E_MIR_UNSUPPORTED_EFFECT");
}

#[test]
fn source_census_rejects_unreported_functions_actors_or_initializers() {
    for source in [
        format!("{}\nfnc hidden(): int {{ return 1; }}\n", valid()),
        format!("declare actor Extra;\n{}", valid()),
        valid().replace("let int xyz = 0;", "let int xyz = helper();"),
        valid().replace("let int xyz = 0;", "let int xyz = 1 + 2;"),
        valid().replace("declare actor Meter;", "declare actor Meter;\nfnc hidden(): int { return 1; }"),
    ] {
        assert!(derive_bounded_guard_mir(&source).is_err(), "{source}");
    }
}
#[test]
fn compound_effect_inventory_is_bounded_before_transport() {
    let base = valid();
    let repeated = "s.counter += 1; ".repeat(257);
    let expanded = base.replace("s.counter += 1;\n        s.xyz;", &repeated);
    let e = derive_bounded_guard_mir(&expanded).expect_err("513 effects must exceed 512 budget");
    assert_eq!(e.code, "E_SOURCE_LIMIT");
}

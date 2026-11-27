//! Incremental compilation v1: content-addressed keys, cache hits,
//! reachability-scoped invalidation and reverse-dependency refresh sets.
use atto_compiler::incremental::{build_key, ImportGraph, IncrementalCompiler, RefreshPlanner};
use atto_compiler::linker::{compile_linked, LinkUnit};
use std::collections::BTreeSet;

const MAIN: &str = r#"
use namespace web;
import module Orders from "+lib/orders.atto";
pub fnc main(): int { return Orders.total(21); }
"#;
const ORDERS: &str = r#"
use namespace sales;
import module Math from "./math.atto";
export pub module Orders;
define module Orders as
    pub fnc total(int x): int { return Math.double(x); }
end
"#;
const MATH: &str = r#"
use namespace maths;
export pub module Math;
define module Math as
    pub fnc double(int x): int { return x * 2; }
end
"#;
const MATH_TRIPLE: &str = r#"
use namespace maths;
export pub module Math;
define module Math as
    pub fnc double(int x): int { return x * 3; }
end
"#;
const UNUSED_A: &str = r#"
use namespace extra;
export pub module Extra;
define module Extra as
    pub fnc one(): int { return 1; }
end
"#;
const UNUSED_B: &str = r#"
use namespace extra;
export pub module Extra;
define module Extra as
    pub fnc one(): int { return 2; }
end
"#;

fn units<'a>(math: &'a str, extra: &'a str) -> Vec<LinkUnit<'a>> {
    vec![
        LinkUnit {
            path: "src/main.atto",
            package: "acme@1.0.0",
            source: MAIN,
        },
        LinkUnit {
            path: "lib/orders.atto",
            package: "acme@1.0.0",
            source: ORDERS,
        },
        LinkUnit {
            path: "lib/math.atto",
            package: "acme@1.0.0",
            source: math,
        },
        LinkUnit {
            path: "lib/extra.atto",
            package: "acme@1.0.0",
            source: extra,
        },
    ]
}

fn graph3(math: &str) -> Vec<LinkUnit<'_>> {
    units(math, UNUSED_A).into_iter().take(3).collect()
}

fn set(items: &[&str]) -> BTreeSet<String> {
    items.iter().map(|s| s.to_string()).collect()
}

#[test]
fn cache_hit_returns_byte_identical_artifact_without_recompiling() {
    let u = graph3(MATH);
    let direct = compile_linked("src/main.atto", &u).expect("fixture compiles");
    let mut c = IncrementalCompiler::default();
    let first = c.compile("src/main.atto", &u).unwrap();
    let second = c.compile("src/main.atto", &u).unwrap();
    assert_eq!(first, direct);
    assert_eq!(second, direct);
    assert_eq!((c.stats().hits, c.stats().misses), (1, 1));
}

#[test]
fn reachable_byte_change_always_invalidates() {
    let mut c = IncrementalCompiler::default();
    let a = c.compile("src/main.atto", &graph3(MATH)).unwrap();
    let b = c.compile("src/main.atto", &graph3(MATH_TRIPLE)).unwrap();
    assert_ne!(a, b, "different guest semantics must produce a different artifact");
    assert_eq!(c.stats().misses, 2);
    assert_ne!(
        build_key("src/main.atto", &graph3(MATH)).unwrap(),
        build_key("src/main.atto", &graph3(MATH_TRIPLE)).unwrap()
    );
}

#[test]
fn every_supplied_unit_is_keyed_because_the_linker_rejects_unreachable_units() {
    // compile_linked's *result* depends on unreachable inputs (E_UNBOUND),
    // so excluding them from the key would make a cache hit unsound.
    let with_extra = units(MATH, UNUSED_A);
    assert_eq!(compile_linked("src/main.atto", &with_extra).unwrap_err().code, "E_UNBOUND");
    assert_ne!(
        build_key("src/main.atto", &with_extra).unwrap(),
        build_key("src/main.atto", &units(MATH, UNUSED_B)).unwrap()
    );
    assert_ne!(
        build_key("src/main.atto", &with_extra).unwrap(),
        build_key("src/main.atto", &graph3(MATH)).unwrap()
    );
}

#[test]
fn key_is_independent_of_unit_order() {
    let mut reversed = graph3(MATH);
    reversed.reverse();
    assert_eq!(
        build_key("src/main.atto", &graph3(MATH)).unwrap(),
        build_key("src/main.atto", &reversed).unwrap()
    );
}

#[test]
fn package_identity_is_part_of_the_key() {
    let mut u = units(MATH, UNUSED_A);
    let before = build_key("src/main.atto", &u).unwrap();
    u[2].package = "acme@1.0.1";
    assert_ne!(before, build_key("src/main.atto", &u).unwrap());
}

#[test]
fn diagnostics_are_cached_and_identical() {
    let broken = r#"
use namespace maths;
export pub module Math;
define module Math as
    pub fnc double(int x): int { return x * ; }
end
"#;
    let u = graph3(broken);
    let mut c = IncrementalCompiler::default();
    let e1 = c.compile("src/main.atto", &u).expect_err("broken source must not emit a guest");
    let e2 = c.compile("src/main.atto", &u).expect_err("cached result must stay an error");
    assert_eq!(e1, e2);
    assert_eq!(c.stats().hits, 1);
}

#[test]
fn edit_refresh_set_is_changed_file_plus_transitive_importers() {
    let u = units(MATH, UNUSED_A);
    let g = ImportGraph::build(&u).unwrap();
    assert_eq!(
        g.affected_by(&set(&["lib/math.atto"])),
        set(&["lib/math.atto", "lib/orders.atto", "src/main.atto"])
    );
    assert_eq!(g.affected_by(&set(&["lib/extra.atto"])), set(&["lib/extra.atto"]));
    assert_eq!(
        g.reachable("src/main.atto").unwrap(),
        set(&["lib/math.atto", "lib/orders.atto", "src/main.atto"])
    );
}

#[test]
fn changed_since_last_reports_edits_additions_and_removals() {
    let mut c = IncrementalCompiler::default();
    assert_eq!(c.changed_since_last(&units(MATH, UNUSED_A)).len(), 4);
    assert!(c.changed_since_last(&units(MATH, UNUSED_A)).is_empty());
    assert_eq!(c.changed_since_last(&units(MATH_TRIPLE, UNUSED_A)), set(&["lib/math.atto"]));
    let fewer = &units(MATH_TRIPLE, UNUSED_A)[..3];
    assert_eq!(c.changed_since_last(fewer), set(&["lib/extra.atto"]));
}

#[test]
fn bounded_cache_evicts_oldest_and_rejects_bad_capacity() {
    assert!(IncrementalCompiler::with_capacity(0).is_err());
    assert!(IncrementalCompiler::with_capacity(4097).is_err());
    let mut c = IncrementalCompiler::with_capacity(1).unwrap();
    c.compile("src/main.atto", &graph3(MATH)).unwrap();
    c.compile("src/main.atto", &graph3(MATH_TRIPLE)).unwrap();
    assert_eq!((c.len(), c.stats().evictions), (1, 1));
    c.compile("src/main.atto", &graph3(MATH)).unwrap();
    assert_eq!(c.stats().misses, 3, "evicted entry must be recompiled");
}

#[test]
fn missing_entry_and_duplicate_paths_fail_closed() {
    let u = units(MATH, UNUSED_A);
    assert_eq!(build_key("src/nope.atto", &u).unwrap_err().code, "E_UNBOUND");
    let mut dup = units(MATH, UNUSED_A);
    dup[3].path = "lib/math.atto";
    assert_eq!(build_key("src/main.atto", &dup).unwrap_err().code, "E_DUPLICATE");
}

#[test]
fn package_rebind_is_reported_as_edit_even_when_source_is_unchanged() {
    let mut c = IncrementalCompiler::default();
    let mut u = graph3(MATH);
    c.changed_since_last(&u);
    u[2].package = "acme@1.0.1";
    assert_eq!(c.changed_since_last(&u), set(&["lib/math.atto"]));
}

#[test]
fn cached_compile_does_not_suppress_borrow_export() {
    let u = graph3(MATH);
    let mut c = IncrementalCompiler::default();
    let expected = c.compile("src/main.atto", &u).unwrap();
    atto_compiler::borrow_export::begin();
    let direct = compile_linked("src/main.atto", &u);
    let direct_records = atto_compiler::borrow_export::finish();
    assert_eq!(direct.unwrap(), expected);
    assert!(!direct_records.is_empty());
    atto_compiler::borrow_export::begin();
    let warm = c.compile("src/main.atto", &u);
    let warm_records = atto_compiler::borrow_export::finish();
    assert_eq!(warm.unwrap(), expected);
    assert_eq!(warm_records.len(), direct_records.len());
    assert_eq!(c.stats().hits, 0);
}

#[test]
fn byte_budget_bypasses_oversize_artifact_without_changing_compilation() {
    let u = graph3(MATH);
    let clean = compile_linked("src/main.atto", &u).unwrap();
    let mut c = IncrementalCompiler::with_limits(64, 1).unwrap();
    assert_eq!(c.compile("src/main.atto", &u).unwrap(), clean);
    assert_eq!(c.compile("src/main.atto", &u).unwrap(), clean);
    assert_eq!((c.len(), c.retained_bytes(), c.stats().hits, c.stats().misses), (0, 0, 0, 2));
    assert!(IncrementalCompiler::with_limits(64, 0).is_err());
}

#[test]
fn byte_budget_evicts_even_below_entry_capacity() {
    let u = graph3(MATH);
    let v = graph3(MATH_TRIPLE);
    let first = compile_linked("src/main.atto", &u).unwrap();
    let second = compile_linked("src/main.atto", &v).unwrap();
    let path_bytes: usize = u.iter().map(|x| x.path.len()).sum();
    let budget = first.len().max(second.len()) + path_bytes + 32;
    let mut c = IncrementalCompiler::with_limits(64, budget).unwrap();
    assert_eq!(c.compile("src/main.atto", &u).unwrap(), first);
    assert_eq!(c.compile("src/main.atto", &v).unwrap(), second);
    assert_eq!(c.len(), 1);
    assert_eq!(c.stats().evictions, 1);
    assert!(c.retained_bytes() <= budget);
    assert_eq!(c.compile("src/main.atto", &v).unwrap(), second);
    assert_eq!(c.stats().hits, 1);
}

#[test]
fn exact_hits_skip_import_graph_validation_but_edits_and_invalid_inputs_do_not() {
    let mut c = IncrementalCompiler::default();
    let u = graph3(MATH);
    let clean = compile_linked("src/main.atto", &u).unwrap();
    assert_eq!(c.compile("src/main.atto", &u).unwrap(), clean);
    assert_eq!(c.graph_validations(), 1);
    for _ in 0..16 {
        assert_eq!(c.compile("src/main.atto", &u).unwrap(), clean);
    }
    let mut reversed = graph3(MATH);
    reversed.reverse();
    assert_eq!(c.compile("src/main.atto", &reversed).unwrap(), clean);
    assert_eq!(c.graph_validations(), 1);
    assert_eq!(c.stats().hits, 17);

    let edited = graph3(MATH_TRIPLE);
    assert_eq!(
        c.compile("src/main.atto", &edited).unwrap(),
        compile_linked("src/main.atto", &edited).unwrap()
    );
    assert_eq!(c.graph_validations(), 2);

    let mut duplicate = graph3(MATH);
    duplicate[2].path = duplicate[1].path;
    let want = build_key("src/main.atto", &duplicate).unwrap_err();
    assert_eq!(c.compile("src/main.atto", &duplicate).unwrap_err(), want);
    assert_eq!(c.graph_validations(), 3);
    assert_eq!(c.compile("src/main.atto", &u).unwrap(), clean);
    assert_eq!(c.graph_validations(), 3);
}

#[test]
fn refresh_planner_rechecks_missing_provider_and_transitive_users() {
    let mut p = RefreshPlanner::default();
    let complete = graph3(MATH);
    assert_eq!(p.update(&complete).unwrap(), set(&["src/main.atto","lib/orders.atto","lib/math.atto"]));
    assert!(p.update(&complete).unwrap().is_empty());
    let missing = &complete[..2];
    assert_eq!(p.update(missing).unwrap(), set(&["src/main.atto","lib/orders.atto","lib/math.atto"]));
    assert_eq!(compile_linked("src/main.atto", missing).unwrap_err().code, "E_UNBOUND");
    assert_eq!(p.update(&complete).unwrap(), set(&["src/main.atto","lib/orders.atto","lib/math.atto"]));
    assert!(compile_linked("src/main.atto", &complete).is_ok());
}

#[test]
fn refresh_planner_rejects_duplicate_without_committing_and_tracks_package_rebind() {
    let mut p = RefreshPlanner::default();
    let mut complete = graph3(MATH);
    p.update(&complete).unwrap();
    let mut duplicate = graph3(MATH);
    duplicate[2].path = "lib/orders.atto";
    assert_eq!(p.update(&duplicate).unwrap_err().code, "E_DUPLICATE");
    assert!(p.update(&complete).unwrap().is_empty());
    complete[2].package = "acme@2.0.0";
    assert_eq!(p.update(&complete).unwrap(), set(&["src/main.atto","lib/orders.atto","lib/math.atto"]));
    assert!(p.update(&complete).unwrap().is_empty());
}

#[test]
fn refresh_planner_reports_removed_edges_and_reordered_inventory() {
    let mut p = RefreshPlanner::default();
    let complete = graph3(MATH);
    p.update(&complete).unwrap();
    let mut reordered = graph3(MATH);
    reordered.reverse();
    assert!(p.update(&reordered).unwrap().is_empty());
    let new_orders = r#"use namespace sales;
export pub module Orders;
define module Orders as
    pub fnc total(int x): int { return x * 2; }
end"#;
    let mut changed = graph3(MATH);
    changed[1].source = new_orders;
    changed.pop();
    assert_eq!(p.update(&changed).unwrap(), set(&["src/main.atto","lib/orders.atto","lib/math.atto"]));
    assert!(compile_linked("src/main.atto", &changed).is_ok());
    assert!(p.update(&changed).unwrap().is_empty());
}

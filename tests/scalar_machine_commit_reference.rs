//! Host-only scalar transition reference; no guest or scheduler proof.
use atto_compiler::parser::{FlatMachineTable, Parser};
use std::{collections::BTreeMap, sync::{Arc, Mutex}};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Snapshot { node: usize, value: i64, version: i64, generation: i64 }
#[derive(Clone, Copy)]
struct Operation { node: usize, event: usize, delta: i64 }
#[derive(Debug, Eq, PartialEq)]
enum Outcome { Committed, Stale, Terminal, NoEdge, InvalidEvent, Cancelled, Overflow, Invariant, Exhausted }

struct Reference {
    table: FlatMachineTable,
    deltas: BTreeMap<(usize, usize), i64>,
    limit: i64,
    current: Mutex<Snapshot>,
}
impl Reference {
    fn new(table: FlatMachineTable, operations: &[Operation], initial: i64, limit: i64) -> Result<Self, &'static str> {
        if initial < 0 || limit < 1 { return Err("invalid initial snapshot or limit"); }
        let mut expected = 0;
        for node in 0..table.names().len() {
            for event in 0..table.events().len() {
                if matches!(table.successor(node, event), Some(Some(_))) { expected += 1; }
            }
        }
        if operations.len() != expected { return Err("incomplete transition plan"); }
        let mut deltas = BTreeMap::new();
        for op in operations {
            if !matches!(table.successor(op.node, op.event), Some(Some(_)))
                || deltas.insert((op.node, op.event), op.delta).is_some() {
                return Err("unknown or duplicate operation");
            }
        }
        let current = Mutex::new(Snapshot { node: table.initial(), value: initial, version: 0, generation: 1 });
        Ok(Self { table, deltas, limit, current })
    }

    fn snapshot(&self) -> Snapshot { *self.current.lock().unwrap() }

    fn apply(&self, version: i64, generation: i64, event: usize, cancelled: bool) -> Outcome {
        let mut current = self.current.lock().unwrap();
        let old = *current;
        if old.version != version || old.generation != generation { return Outcome::Stale; }
        if self.table.terminal(old.node) == Some(true) { return Outcome::Terminal; }
        let next = match self.table.successor(old.node, event) {
            None => return Outcome::InvalidEvent,
            Some(None) => return Outcome::NoEdge,
            Some(Some(target)) => target,
        };
        if cancelled { return Outcome::Cancelled; }
        if old.version >= self.limit || old.generation >= self.limit { return Outcome::Exhausted; }
        let Some(value) = old.value.checked_add(self.deltas[&(old.node, event)]) else { return Outcome::Overflow; };
        if value < 0 { return Outcome::Invariant; }
        *current = Snapshot { node: next, value, version: old.version + 1, generation: old.generation + 1 };
        Outcome::Committed
    }
}
fn table() -> FlatMachineTable {
    let source = "define actor Counter as state machine as initial state Idle; final state Complete; state Idle as on Add() -> Idle; on Finish() -> Complete; end end end";
    Parser::inspect_actor_machines(source).unwrap()[0].machine.as_ref().unwrap().transition_table().unwrap()
}
fn machine(initial: i64, limit: i64, delta: i64) -> Reference {
    Reference::new(table(), &[Operation { node: 1, event: 0, delta }, Operation { node: 1, event: 1, delta: 0 }], initial, limit).unwrap()
}

#[test]
fn scalar_commit_and_rejection() {
    let m = machine(1, i64::MAX, 2);
    let start = m.snapshot();
    assert_eq!(start, Snapshot { node: 1, value: 1, version: 0, generation: 1 });
    assert_eq!(m.apply(0, 1, 0, true), Outcome::Cancelled);
    assert_eq!(m.snapshot(), start);
    assert_eq!(m.apply(0, 1, usize::MAX, false), Outcome::InvalidEvent);
    assert_eq!(m.snapshot(), start);
    assert_eq!(m.apply(0, 1, 0, false), Outcome::Committed);
    let after = m.snapshot();
    assert_eq!(after, Snapshot { node: 1, value: 3, version: 1, generation: 2 });
    assert_eq!(m.apply(0, 1, 0, false), Outcome::Stale);
    assert_eq!(m.apply(1, 1, 0, false), Outcome::Stale);
    assert_eq!(m.snapshot(), after);
    assert_eq!(m.apply(1, 2, 1, false), Outcome::Committed);
    assert_eq!(m.snapshot(), Snapshot { node: 0, value: 3, version: 2, generation: 3 });
    assert_eq!(m.apply(2, 3, 0, false), Outcome::Terminal);
    let negative = machine(1, 20, -2);
    assert_eq!(negative.apply(0, 1, 0, false), Outcome::Invariant);
    assert_eq!(negative.snapshot(), start);
    let overflow = machine(i64::MAX, 20, 1);
    assert_eq!(overflow.apply(0, 1, 0, false), Outcome::Overflow);
    assert_eq!(overflow.snapshot().value, i64::MAX);
    assert_eq!(overflow.snapshot().version, 0);
    let exhausted = machine(1, 2, 1);
    assert_eq!(exhausted.apply(0, 1, 0, false), Outcome::Committed);
    let last = exhausted.snapshot();
    assert_eq!(exhausted.apply(1, 2, 0, false), Outcome::Exhausted);
    assert_eq!(exhausted.snapshot(), last);
}
#[test]
fn incomplete_duplicate_and_unknown_plans_rejected() {
    assert!(Reference::new(table(), &[], 0, 1).is_err());
    assert!(Reference::new(table(), &[Operation { node: 1, event: 0, delta: 1 }, Operation { node: 1, event: 0, delta: 2 }], 0, 1).is_err());
    assert!(Reference::new(table(), &[Operation { node: 1, event: 0, delta: 1 }, Operation { node: 0, event: 1, delta: 2 }], 0, 1).is_err());
}
#[test]
fn same_version_race_commits_exactly_once() {
    let m = Arc::new(machine(1, 20, 1));
    let barrier = Arc::new(std::sync::Barrier::new(2));
    let run = || {
        let m = Arc::clone(&m);
        let barrier = Arc::clone(&barrier);
        std::thread::spawn(move || { barrier.wait(); m.apply(0, 1, 0, false) })
    };
    let a = run();
    let b = run();
    let outcomes = [a.join().unwrap(), b.join().unwrap()];
    assert_eq!(outcomes.iter().filter(|x| **x == Outcome::Committed).count(), 1);
    assert_eq!(outcomes.iter().filter(|x| **x == Outcome::Stale).count(), 1);
    assert_eq!(m.snapshot(), Snapshot { node: 1, value: 2, version: 1, generation: 2 });
}

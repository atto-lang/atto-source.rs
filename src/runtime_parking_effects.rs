//! Internal effect closure over already-resolved callable IDs. No source
//! spelling recognition, guest admission, or continuation codegen here.
use std::collections::VecDeque;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ParkingEffects {
    pub explicit_suspend: bool,
    pub runtime_park: bool,
    pub unresolved: bool,
}
impl ParkingEffects {
    pub fn requires_resumable_frame(self) -> bool {
        self.explicit_suspend || self.runtime_park || self.unresolved
    }
    pub fn requires_suspend_annotation(self) -> bool {
        self.explicit_suspend
    }
    pub fn safe_with_exclusive_loan(self) -> bool {
        !self.requires_resumable_frame()
    }
    fn union(self, other: Self) -> Self {
        Self {
            explicit_suspend: self.explicit_suspend || other.explicit_suspend,
            runtime_park: self.runtime_park || other.runtime_park,
            unresolved: self.unresolved || other.unresolved,
        }
    }
}

/// None denotes an unresolved/foreign/dynamic call; never proves nonparking.
/// The frontend is responsible for producing real resolved IDs, not names.
pub struct CallableParking {
    pub direct: ParkingEffects,
    pub calls: Vec<Option<usize>>,
}

pub fn infer_parking(nodes: &[CallableParking]) -> Result<Vec<ParkingEffects>, &'static str> {
    if nodes.len() > 1024 {
        return Err("parking graph callable budget exceeded");
    }
    let mut edges = 0usize;
    for node in nodes {
        if node.calls.len() > 1024 || node.calls.len() > 16384 - edges {
            return Err("parking graph edge budget exceeded");
        }
        edges += node.calls.len();
    }
    let mut reverse = vec![Vec::new(); nodes.len()];
    let mut effects: Vec<_> = nodes.iter().map(|n| n.direct).collect();
    for (caller, node) in nodes.iter().enumerate() {
        for target in &node.calls {
            match *target {
                Some(callee) if callee < nodes.len() => reverse[callee].push(caller),
                Some(_) => return Err("invalid resolved parking graph target"),
                None => effects[caller].unresolved = true,
            }
        }
    }
    // Monotonic 3-bit lattice: each row changes at most three times.
    let mut queue: VecDeque<_> = (0..nodes.len()).collect();
    let mut enqueued = vec![true; nodes.len()];
    while let Some(callee) = queue.pop_front() {
        enqueued[callee] = false;
        for &caller in &reverse[callee] {
            let next = effects[caller].union(effects[callee]);
            if next != effects[caller] {
                effects[caller] = next;
                if !enqueued[caller] {
                    queue.push_back(caller);
                    enqueued[caller] = true;
                }
            }
        }
    }
    Ok(effects)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn node(direct: ParkingEffects, calls: Vec<Option<usize>>) -> CallableParking {
        CallableParking { direct, calls }
    }
    #[test]
    fn runtime_parking_propagates_through_recursive_callers_without_annotation() {
        let empty = ParkingEffects::default();
        let park = ParkingEffects { runtime_park: true, ..empty };
        let rows = infer_parking(&[
            node(empty, vec![Some(1)]),
            node(empty, vec![Some(0), Some(2)]),
            node(park, vec![]),
            node(empty, vec![]),
        ]).unwrap();
        for row in &rows[..3] {
            assert!(row.requires_resumable_frame());
            assert!(!row.requires_suspend_annotation());
            assert!(!row.safe_with_exclusive_loan());
        }
        assert!(rows[3].safe_with_exclusive_loan());
    }
    #[test]
    fn explicit_and_unresolved_are_not_laundered() {
        let empty = ParkingEffects::default();
        let rows = infer_parking(&[
            node(ParkingEffects { explicit_suspend: true, ..empty }, vec![]),
            node(ParkingEffects { runtime_park: true, ..empty }, vec![Some(0)]),
            node(empty, vec![None]),
            node(empty, vec![Some(2)]),
        ]).unwrap();
        assert!(rows[1].explicit_suspend && rows[1].runtime_park);
        assert!(rows[1].requires_suspend_annotation());
        assert!(rows[3].unresolved);
        assert!(!rows[3].safe_with_exclusive_loan());
        assert!(!rows[3].requires_suspend_annotation());
    }
    #[test]
    fn rejects_invalid_ids_and_graph_budgets() {
        let empty = ParkingEffects::default();
        assert!(infer_parking(&[node(empty, vec![Some(1)])]).is_err());
        assert!(infer_parking(&[node(empty, vec![None; 1025])]).is_err());
        let too_many: Vec<_> = (0..1025).map(|_| node(empty, vec![])).collect();
        assert!(infer_parking(&too_many).is_err());
        let too_wide: Vec<_> = (0..17).map(|_| node(empty, vec![None; 1024])).collect();
        assert!(infer_parking(&too_wide).is_err());
    }
    #[test]
    fn differential_reachability_oracle_and_exact_budget() {
        fn next(state: &mut u64) -> usize {
            *state = state.wrapping_mul(6364136223846793005).wrapping_add(1);
            (*state >> 32) as usize
        }
        let mut seed = 1127u64;
        for _ in 0..256 {
            let n = 1 + next(&mut seed) % 48;
            let nodes: Vec<_> = (0..n).map(|_| {
                let bits = next(&mut seed) % 16;
                let direct = ParkingEffects {
                    explicit_suspend: bits == 1,
                    runtime_park: bits == 2,
                    unresolved: bits == 3,
                };
                let calls = (0..next(&mut seed) % 8).map(|_| {
                    let target = next(&mut seed) % (n + 3);
                    if target < n { Some(target) } else { None }
                }).collect();
                node(direct, calls)
            }).collect();
            let actual = infer_parking(&nodes).unwrap();
            for root in 0..n {
                let mut expected = ParkingEffects::default();
                let mut seen = vec![false; n];
                let mut pending = vec![root];
                while let Some(i) = pending.pop() {
                    if seen[i] { continue; }
                    seen[i] = true;
                    expected.explicit_suspend |= nodes[i].direct.explicit_suspend;
                    expected.runtime_park |= nodes[i].direct.runtime_park;
                    expected.unresolved |= nodes[i].direct.unresolved;
                    for target in &nodes[i].calls {
                        if let Some(t) = target { pending.push(*t); }
                        else { expected.unresolved = true; }
                    }
                }
                assert_eq!(actual[root], expected);
                assert_eq!(actual[root].requires_suspend_annotation(), expected.explicit_suspend);
            }
        }
        let empty = ParkingEffects::default();
        let park = ParkingEffects { runtime_park: true, ..empty };
        let cycle: Vec<_> = (0..1024).map(|i| node(
            if i == 1023 { park } else { empty },
            vec![Some((i + 1) % 1024); 16],
        )).collect();
        assert!(infer_parking(&cycle).unwrap().iter().all(|r| *r == park));
    }

}

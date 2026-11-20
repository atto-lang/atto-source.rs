//! A finite, deterministic typed-IR graph audit for actor DATA send effects.
//! This module is shared byte-identically by the Rust/Wasm and LLVM-repository
//! Rust frontends. It DOES NOT authorize or route supervisor CONTROL messages.
#![forbid(unsafe_code)]

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EffectNode<T> {
    pub sends: Vec<T>,
    pub calls: Vec<u32>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EffectError {
    OutboundTypeViolation,
    UnknownCallTarget,
}

/// Audit all function bodies reachable from actor handlers/methods and from
/// every actor field initializer. Reaching a helper through an initializer is
/// NOT an escape hatch. This graph already represents fully resolved typed IR:
/// the compiler must reject dynamic/erased call targets before building it.
///
/// An explicit Never bound is None. A Payload(T) bound is Some(T). Absence of
/// a source-level sends clause is deliberately NOT handled by this function:
/// callers must infer and publish that effect separately before claiming strict
/// typed actor interface parity.
pub fn audit_effect<T: Copy + Eq>(
    allowed: Option<T>,
    functions: &[EffectNode<T>],
    entrypoints: &[u32],
    initializers: &[EffectNode<T>],
) -> Result<(), EffectError> {
    // Inspect all initializers, including those whose results are unused.
    for init in initializers {
        if init.sends.iter().any(|v| Some(*v) != allowed) {
            return Err(EffectError::OutboundTypeViolation);
        }
    }
    // Reject any forged/missing index even if an earlier allowed path would
    // otherwise look harmless. Visit on enqueue to bound traversals of cycles
    // and dense call graphs to at most one scan per compiled function.
    let mut seen = vec![false; functions.len()];
    let mut work = Vec::<usize>::new();
    for id in entrypoints.iter().copied()
        .chain(initializers.iter().flat_map(|node| node.calls.iter().copied()))
    {
        let index = usize::try_from(id).map_err(|_| EffectError::UnknownCallTarget)?;
        if index >= functions.len() {
            return Err(EffectError::UnknownCallTarget);
        }
        if !seen[index] {
            seen[index] = true;
            work.push(index);
        }
    }
    while let Some(index) = work.pop() {
        let node = &functions[index];
        if node.sends.iter().any(|v| Some(*v) != allowed) {
            return Err(EffectError::OutboundTypeViolation);
        }
        for id in &node.calls {
            let target = usize::try_from(*id).map_err(|_| EffectError::UnknownCallTarget)?;
            if target >= functions.len() {
                return Err(EffectError::UnknownCallTarget);
            }
            if !seen[target] {
                seen[target] = true;
                work.push(target);
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn node(sends: &[u8], calls: &[u32]) -> EffectNode<u8> {
        EffectNode { sends: sends.to_vec(), calls: calls.to_vec() }
    }

    #[test]
    fn never_effect_accepts_no_application_sends() {
        assert_eq!(audit_effect::<u8>(None, &[node(&[], &[])], &[0], &[]), Ok(()));
    }

    #[test]
    fn never_rejects_direct_send() {
        assert_eq!(audit_effect(None, &[node(&[1], &[])], &[0], &[]),
            Err(EffectError::OutboundTypeViolation));
    }

    #[test]
    fn never_rejects_send_through_multiple_helpers() {
        let graph = [node(&[], &[1]), node(&[], &[2]), node(&[1], &[])];
        assert_eq!(audit_effect(None, &graph, &[0], &[]),
            Err(EffectError::OutboundTypeViolation));
    }

    #[test]
    fn never_rejects_direct_initializer_send() {
        assert_eq!(audit_effect(None, &[], &[], &[node(&[1], &[])]),
            Err(EffectError::OutboundTypeViolation));
    }

    #[test]
    fn never_rejects_helper_send_from_initializer() {
        let graph = [node(&[], &[1]), node(&[2], &[])];
        assert_eq!(audit_effect(None, &graph, &[], &[node(&[], &[0])]),
            Err(EffectError::OutboundTypeViolation));
    }

    #[test]
    fn declared_payload_allows_matching_calls_and_initializers() {
        let graph = [node(&[], &[1]), node(&[1], &[0])]; // cycle
        assert_eq!(audit_effect(Some(1), &graph, &[0],
            &[node(&[1], &[1])]), Ok(()));
    }

    #[test]
    fn declared_payload_rejects_mixed_type() {
        let graph = [node(&[], &[1]), node(&[2], &[])];
        assert_eq!(audit_effect(Some(1), &graph, &[0], &[]),
            Err(EffectError::OutboundTypeViolation));
    }

    #[test]
    fn initializer_only_call_path_is_checked_even_when_handler_is_safe() {
        let graph = [node(&[], &[]), node(&[], &[2]), node(&[9], &[])];
        assert_eq!(audit_effect(Some(1), &graph, &[0], &[node(&[], &[1])]),
            Err(EffectError::OutboundTypeViolation));
    }

    #[test]
    fn reachable_missing_callee_is_rejected() {
        let graph = [node(&[], &[2])];
        assert_eq!(audit_effect(Some(1), &graph, &[0], &[]),
            Err(EffectError::UnknownCallTarget));
        assert_eq!(audit_effect(Some(1), &graph, &[4], &[]),
            Err(EffectError::UnknownCallTarget));
        assert_eq!(audit_effect(Some(1), &graph, &[], &[node(&[], &[4])]),
            Err(EffectError::UnknownCallTarget));
    }

    #[test]
    fn deeply_cyclic_helpers_terminate_without_recursive_stack() {
        let mut graph = Vec::new();
        for i in 0..2048u32 {
            graph.push(node(&[], &[if i == 2047 { 0 } else { i + 1 }]));
        }
        assert_eq!(audit_effect(None, &graph, &[0], &[]), Ok(()));
        graph[1536].sends.push(1);
        assert_eq!(audit_effect(None, &graph, &[0], &[]),
            Err(EffectError::OutboundTypeViolation));
    }

    #[test]
    fn unreachable_ordinary_helpers_are_not_actor_effects() {
        let graph = [node(&[], &[]), node(&[2], &[])];
        assert_eq!(audit_effect(Some(1), &graph, &[0], &[]), Ok(()));
    }

    #[test]
    fn inferred_effect_is_not_silently_interpreted_as_never_or_any() {
        // Caller must not pass None for an omitted sends clause: this helper
        // has no inference entry point. Explicit never is audited only.
        assert_eq!(audit_effect::<u8>(None, &[], &[], &[]), Ok(()));
    }
}

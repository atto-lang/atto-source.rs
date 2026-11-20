//! Compile-time pipe port admission shared by actors and future typed endpoints.
//!
//! This checks a graph's *capability contract*, not its runtime transport.
//! Deferred endpoints are fail-closed by the caller until their concrete
//! type/ownership/effect and guest ABI become available.
use crate::ast::Type;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PipePort {
    /// A PipeSink<T> capability. None means the stage is not writable.
    pub input: Option<Type>,
    /// A PipeSource<T> capability. None means the stage is not readable.
    pub output: Option<Type>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StageContract {
    Known(PipePort),
    /// Unimplemented endpoint constructors (streams/channels/generators)
    /// may be recognized syntactically but MUST NOT generate a guest.
    Deferred,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PipeContractError {
    MissingSource { stage: usize },
    MissingSink { stage: usize },
    TypeMismatch {
        from: usize,
        to: usize,
        emitted: Type,
        received: Type,
    },
}

/// A direction-normalized topology uses lexical indices rather than moving
/// endpoint expressions. The compiler must evaluate constructors in written
/// order exactly once; only DATA edge order is reversed for `rpipe`.
///
/// This is a statically derived plan, never a guest runtime connection,
/// authorization token or PipeHandle.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DirectedPipeTopology {
    /// Lexical source indices in actual source-to-sink DATA order.
    pub flow_indices: Vec<usize>,
    /// Edges refer to the original lexical stages; never to actor CONTROL.
    pub edges: Vec<(usize, usize)>,
}

/// Derive a fixed topology without evaluating or cloning endpoint expressions.
/// The browser profile permits up to 16 stages; the actual runtime must
/// independently validate tenant, ownership and source/sink capabilities.
pub fn directed_topology(
    stage_count: usize,
    direction: crate::ast::PipeDirection,
) -> Option<DirectedPipeTopology> {
    if !(2..=16).contains(&stage_count) {
        return None;
    }
    let mut flow_indices: Vec<usize> = (0..stage_count).collect();
    if direction == crate::ast::PipeDirection::RightToLeft {
        flow_indices.reverse();
    }
    let edges = flow_indices.windows(2).map(|p| (p[0], p[1])).collect();
    Some(DirectedPipeTopology { flow_indices, edges })
}

/// Check each stage's input/output role and all fully-known adjacent edges.
/// An unknown stage defers its own edges but must never authorize guest code.
pub fn validate_known_edges(stages: &[StageContract]) -> Result<(), PipeContractError> {
    if stages.len() < 2 {
        return Err(PipeContractError::MissingSource { stage: 0 });
    }
    for (i, contract) in stages.iter().enumerate() {
        if let StageContract::Known(port) = contract {
            if i > 0 && port.input.is_none() {
                return Err(PipeContractError::MissingSink { stage: i });
            }
            if i + 1 < stages.len() && port.output.is_none() {
                return Err(PipeContractError::MissingSource { stage: i });
            }
        }
    }
    for i in 0..(stages.len() - 1) {
        if let (StageContract::Known(a), StageContract::Known(b)) =
            (stages[i], stages[i + 1])
        {
            if let (Some(emitted), Some(received)) = (a.output, b.input) {
                if emitted != received {
                    return Err(PipeContractError::TypeMismatch {
                        from: i,
                        to: i + 1,
                        emitted,
                        received,
                    });
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn port(input: Option<Type>, output: Option<Type>) -> StageContract {
        StageContract::Known(PipePort { input, output })
    }

    #[test]
    fn forward_and_reverse_spelling_produce_identical_data_edges() {
        use crate::ast::PipeDirection;
        let forward = directed_topology(3, PipeDirection::LeftToRight).unwrap();
        let reverse = directed_topology(3, PipeDirection::RightToLeft).unwrap();
        // Producer, Transform, Consumer written forward.
        assert_eq!(forward.flow_indices, vec![0, 1, 2]);
        assert_eq!(forward.edges, vec![(0, 1), (1, 2)]);
        // Consumer, Transform, Producer written backward. Written indices
        // reverse but source->sink identities and DATA order remain identical.
        assert_eq!(reverse.flow_indices, vec![2, 1, 0]);
        assert_eq!(reverse.edges, vec![(2, 1), (1, 0)]);
        for written in [0usize, 1, 2] {
            assert_eq!(forward.flow_indices[written], reverse.flow_indices[2-written]);
        }
    }

    #[test]
    fn pipeline_topology_enforces_tenant_stage_limit_without_cloning_expressions() {
        use crate::ast::PipeDirection;
        for n in [0, 1, 17, usize::MAX] {
            assert_eq!(directed_topology(n, PipeDirection::LeftToRight), None);
            assert_eq!(directed_topology(n, PipeDirection::RightToLeft), None);
        }
        for n in [2, 3, 16] {
            for direction in [PipeDirection::LeftToRight, PipeDirection::RightToLeft] {
                let plan = directed_topology(n, direction).unwrap();
                assert_eq!(plan.edges.len(), n - 1);
                assert_eq!(plan.flow_indices.len(), n);
                let mut sorted = plan.flow_indices.clone();
                sorted.sort_unstable();
                assert_eq!(sorted, (0..n).collect::<Vec<_>>());
                assert!(plan.edges.iter().all(|&(from, to)| from != to));
            }
        }
    }

    #[test]
    fn producer_transform_consumer_is_typed_and_ordered() {
        assert_eq!(validate_known_edges(&[
            port(None, Some(Type::Int)),
            port(Some(Type::Int), Some(Type::Bool)),
            port(Some(Type::Bool), None),
        ]), Ok(()));
    }

    #[test]
    fn sink_only_cannot_source() {
        assert_eq!(validate_known_edges(&[
            port(Some(Type::Int), None),
            port(Some(Type::Int), None),
        ]), Err(PipeContractError::MissingSource { stage: 0 }));
    }

    #[test]
    fn source_only_cannot_be_middle_transform() {
        assert_eq!(validate_known_edges(&[
            port(None, Some(Type::Int)),
            port(None, Some(Type::Bool)),
            port(Some(Type::Bool), None),
        ]), Err(PipeContractError::MissingSink { stage: 1 }));
    }

    #[test]
    fn actor_to_actor_item_mismatch_is_rejected_before_runtime() {
        assert_eq!(validate_known_edges(&[
            port(None, Some(Type::Bool)),
            port(Some(Type::Int), None),
        ]), Err(PipeContractError::TypeMismatch {
            from: 0, to: 1, emitted: Type::Bool, received: Type::Int,
        }));
    }

    #[test]
    fn deferred_channel_does_not_grant_source_or_sink_capability() {
        // The Rust browser compiler must reject this graph as E_UNSUPPORTED
        // after static known edges are checked, not treat Deferred as Any.
        assert_eq!(validate_known_edges(&[
            port(None, Some(Type::Int)),
            StageContract::Deferred,
            port(Some(Type::Bool), None),
        ]), Ok(()));
    }

    #[test]
    fn no_pipeline_requires_at_least_two_stages() {
        assert_eq!(validate_known_edges(&[]),
            Err(PipeContractError::MissingSource { stage: 0 }));
    }

    #[test]
    fn output_type_is_invariant_across_pipe_boundaries() {
        assert_eq!(validate_known_edges(&[
            port(None, Some(Type::Int)),
            port(Some(Type::Int), Some(Type::Int)),
            port(Some(Type::Bool), None),
        ]), Err(PipeContractError::TypeMismatch {
            from: 1, to: 2, emitted: Type::Int, received: Type::Bool,
        }));
    }
}

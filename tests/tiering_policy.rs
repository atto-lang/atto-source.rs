use oreslang_source_rs::profile::BuildProfile;
use oreslang_source_rs::ref_layout::LayoutKind;
use oreslang_source_rs::tiering::{
    recommend_layout, DeploymentTarget, ExecutionPlan, LayoutEvidence, OptimizationMode,
    PlanError, TraceCounters, TraceEvent, TrustLevel, TypeShape,
};

#[test]
fn native_trusted_jit_request_is_admitted_as_a_policy_only() {
    for target in [DeploymentTarget::NativeServer, DeploymentTarget::NativeDesktop] {
        let plan = ExecutionPlan::admit(
            target,
            BuildProfile::Dev,
            OptimizationMode::AdaptiveJit,
            TrustLevel::Trusted,
            true,
        )
        .unwrap();
        assert!(plan.aot_baseline_required);
        assert_eq!(plan.optimization, OptimizationMode::AdaptiveJit);
    }
}

#[test]
fn strict_aot_and_hard_rt_cannot_request_runtime_jit() {
    for profile in [BuildProfile::StrictAot, BuildProfile::HardRt] {
        assert_eq!(
            ExecutionPlan::admit(
                DeploymentTarget::NativeServer,
                profile,
                OptimizationMode::AdaptiveJit,
                TrustLevel::Trusted,
                true,
            ),
            Err(PlanError::JitForbiddenByProfile)
        );
    }
}

#[test]
fn browser_wasi_and_embedded_never_admit_runtime_jit() {
    for target in [
        DeploymentTarget::BrowserWasm,
        DeploymentTarget::Wasi,
        DeploymentTarget::Embedded,
    ] {
        assert_eq!(
            ExecutionPlan::admit(
                target,
                BuildProfile::Release,
                OptimizationMode::AdaptiveJit,
                TrustLevel::Trusted,
                true,
            ),
            Err(PlanError::JitForbiddenByTarget)
        );
    }
}

#[test]
fn untrusted_or_no_exec_memory_rejects_jit() {
    for (trust, executable_memory_allowed) in [
        (TrustLevel::Untrusted, true),
        (TrustLevel::Trusted, false),
        (TrustLevel::Untrusted, false),
    ] {
        assert_eq!(
            ExecutionPlan::admit(
                DeploymentTarget::NativeDesktop,
                BuildProfile::Release,
                OptimizationMode::AdaptiveJit,
                trust,
                executable_memory_allowed,
            ),
            Err(PlanError::JitRequiresTrustedHost)
        );
    }
}

#[test]
fn offline_pgo_and_aot_keep_baseline_on_all_targets() {
    for target in [
        DeploymentTarget::NativeServer,
        DeploymentTarget::NativeDesktop,
        DeploymentTarget::BrowserWasm,
        DeploymentTarget::Wasi,
        DeploymentTarget::Embedded,
    ] {
        for mode in [OptimizationMode::Aot, OptimizationMode::ProfileGuidedAot] {
            let plan = ExecutionPlan::admit(
                target,
                BuildProfile::StrictAot,
                mode,
                TrustLevel::Untrusted,
                false,
            )
            .unwrap();
            assert!(plan.aot_baseline_required);
        }
    }
}

#[test]
fn trace_observes_exact_threshold_once() {
    let mut counters = TraceCounters::<2>::new(3).unwrap();
    assert_eq!(counters.observe(1), Ok(TraceEvent::Cold));
    assert_eq!(counters.observe(1), Ok(TraceEvent::Cold));
    assert_eq!(counters.observe(1), Ok(TraceEvent::BecameHot));
    assert_eq!(counters.observe(1), Ok(TraceEvent::AlreadyHot));
    assert_eq!(counters.count(1), Some(4));
    assert_eq!(counters.count(0), Some(0));
}

#[test]
fn trace_rejects_invalid_capacity_index_and_zero_threshold() {
    assert_eq!(TraceCounters::<1>::new(0), Err(PlanError::InvalidThreshold));
    let mut empty = TraceCounters::<0>::new(1).unwrap();
    assert_eq!(empty.observe(0), Err(PlanError::TraceSlotOutOfRange));
    let mut one = TraceCounters::<1>::new(1).unwrap();
    assert_eq!(one.observe(usize::MAX), Err(PlanError::TraceSlotOutOfRange));
    assert_eq!(one.count(1), None);
}

#[test]
fn inverted_class_chooses_one_column_representation_without_evidence() {
    assert_eq!(
        recommend_layout(TypeShape::InvertedClass, LayoutEvidence::default(), None),
        Ok(LayoutKind::Soa)
    );
    assert_eq!(
        recommend_layout(TypeShape::InvertedClass, LayoutEvidence::default(), Some(8)),
        Ok(LayoutKind::Aosoa { lanes: 8 })
    );
}

#[test]
fn ordinary_class_requires_evidence_and_is_row_first() {
    assert_eq!(
        recommend_layout(TypeShape::OrdinaryClass, LayoutEvidence::default(), None),
        Err(PlanError::NoLayoutEvidence)
    );
    assert_eq!(
        recommend_layout(
            TypeShape::OrdinaryClass,
            LayoutEvidence {
                row_oriented_reads: 100,
                column_oriented_reads: 150,
            },
            Some(8)
        ),
        Ok(LayoutKind::Aos)
    );
}

#[test]
fn column_advantage_selects_soa_or_aosoa() {
    let evidence = LayoutEvidence {
        row_oriented_reads: 50,
        column_oriented_reads: 100,
    };
    assert_eq!(
        recommend_layout(TypeShape::OrdinaryClass, evidence, None),
        Ok(LayoutKind::Soa)
    );
    assert_eq!(
        recommend_layout(TypeShape::OrdinaryClass, evidence, Some(16)),
        Ok(LayoutKind::Aosoa { lanes: 16 })
    );
}

#[test]
fn layout_ratio_avoids_overflow_near_u64_max() {
    let evidence = LayoutEvidence {
        row_oriented_reads: u64::MAX,
        column_oriented_reads: u64::MAX,
    };
    assert_eq!(
        recommend_layout(TypeShape::OrdinaryClass, evidence, None),
        Ok(LayoutKind::Aos)
    );
}

#[test]
fn lane_admission_rejects_zero_nonpowers_and_oversized_lanes() {
    for lanes in [0, 1, 3, 65, usize::MAX] {
        assert_eq!(
            recommend_layout(TypeShape::InvertedClass, LayoutEvidence::default(), Some(lanes)),
            Err(PlanError::InvalidVectorLanes)
        );
    }
}

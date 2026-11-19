use atto_compiler::proxy_receipt_gate_v1::{
    ProxyReceiptGateV1, ReceiptIdentityV1 as Id, ReceiptEventV1 as Ev,
    ReceiptStageV1 as Stage, ReceiptOutcomeV1 as Outcome,
};
fn id() -> Id {
    Id { protocol_major: 1, tenant: 11, caller_incarnation: 12,
        target_actor: 13, target_incarnation: 14, placement_epoch: 15,
        message_id: 16, attempt: 1 }
}
#[test]
fn remote_proxy_ack_gate_is_not_local_enqueue() {
    let i = id();
    let mut g = ProxyReceiptGateV1::new(i);
    assert!(!g.destination_admitted());
    for e in [Ev::Started, Ev::Processed, Ev::DurableInbox, Ev::DurableResult] {
        assert!(!g.apply_verified(i, e));
    }
    for bad in [Id { tenant: 99, ..i },
                Id { caller_incarnation: 99, ..i },
                Id { target_actor: 99, ..i },
                Id { target_incarnation: 99, ..i },
                Id { placement_epoch: 99, ..i },
                Id { attempt: 99, ..i },
                Id { protocol_major: 2, ..i }] {
        assert!(!g.apply_verified(bad, Ev::Admitted));
    }
    assert!(g.apply_verified(i, Ev::Admitted));
    assert!(g.destination_admitted());
    assert!(!g.apply_verified(i, Ev::DurableResult));
    assert!(g.apply_verified(i, Ev::DurableInbox));
    assert!(g.durable_inbox());
    assert_eq!(g.stage(), Stage::Admitted); // inbox durable != processed
    assert!(g.apply_verified(i, Ev::Started));
    assert!(g.apply_verified(i, Ev::Admitted)); // repeat, no regression
    assert_eq!(g.stage(), Stage::Started);
    assert!(g.apply_verified(i, Ev::Processed));
    assert!(g.apply_verified(i, Ev::DurableResult));
    assert!(g.durable_result());
    assert!(!g.apply_verified(i, Ev::Indeterminate));
    assert!(!g.apply_verified(i, Ev::Failed));
}
#[test]
fn lost_connection_is_indeterminate_and_stale_acks_cannot_resolve_it() {
    let i = id();
    let mut g = ProxyReceiptGateV1::new(i);
    assert!(g.apply_verified(i, Ev::Indeterminate));
    assert_eq!(g.outcome(), Outcome::Indeterminate);
    assert!(!g.apply_verified(i, Ev::Admitted));
    assert!(!g.apply_verified(i, Ev::Rejected));
}
#[test]
fn negative_processing_is_not_unsuccessful_destination_admission() {
    let i = id();
    let mut rejected = ProxyReceiptGateV1::new(i);
    assert!(rejected.apply_verified(i, Ev::Rejected));
    assert_eq!(rejected.outcome(), Outcome::Rejected);
    assert!(!rejected.apply_verified(i, Ev::Admitted));
    let mut failed = ProxyReceiptGateV1::new(i);
    assert!(failed.apply_verified(i, Ev::Admitted));
    assert!(!failed.apply_verified(i, Ev::Rejected));
    assert!(failed.apply_verified(i, Ev::Failed));
    assert_eq!(failed.outcome(), Outcome::Failed);
    assert!(!failed.apply_verified(i, Ev::Processed));
}
#[test]
fn invalid_contexts_fail_closed() {
    let i = Id { tenant: 0, ..id() };
    let mut g = ProxyReceiptGateV1::new(i);
    assert!(!g.apply_verified(i, Ev::Admitted));
    let i = Id { attempt: 0, ..id() };
    let mut g = ProxyReceiptGateV1::new(i);
    assert!(!g.apply_verified(i, Ev::Admitted));
}

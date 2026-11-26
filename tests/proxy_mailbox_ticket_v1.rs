#[path = "../src/proxy_receipt_gate_v1.rs"]
mod proxy_receipt_gate_v1;
#[path = "../src/proxy_mailbox_ticket_v1.rs"]
mod proxy_mailbox_ticket_v1;
use proxy_receipt_gate_v1::ReceiptIdentityV1 as Id;
use proxy_mailbox_ticket_v1::{Admission, HostProxyMailboxTicketsV1 as Mailbox, TicketState};
use std::sync::Arc;

fn route(message_id: u64, incarnation: u64) -> Id {
    Id { protocol_major: 1, tenant: 11, caller_incarnation: 12,
        target_actor: 13, target_incarnation: incarnation,
        association_generation: 17, placement_epoch: 15, message_id, attempt: 1 }
}
fn ticket(m: &Mailbox, r: Id, value: i64) -> proxy_mailbox_ticket_v1::HostTicket {
    match m.admit(r, value) {
        Admission::Admitted(t) => t,
        _ => panic!("expected exact message admission"),
    }
}
#[test]
fn exact_mail_not_other_actor_completion_and_backpressure() {
    let a = Mailbox::new(11, 13, 14, 1).unwrap();
    let b = Mailbox::new(11, 13, 14, 1).unwrap();
    let t1 = ticket(&a, route(101, 14), 11);
    let first = a.dispatch().unwrap();
    let queued = ticket(&a, route(102, 14), 12);
    assert!(matches!(a.admit(route(103, 14), 99), Admission::Full));
    let other = ticket(&b, route(104, 14), 70);
    let other_turn = b.dispatch().unwrap();
    assert!(b.commit(&other_turn, 140));
    assert_eq!(b.snapshot(&other).unwrap().result, Some(140));
    assert_eq!(a.snapshot(&t1).unwrap().state, TicketState::Started);
    assert_eq!(a.snapshot(&queued).unwrap().state, TicketState::Admitted);
    assert!(!a.commit(&other_turn, 99));
    assert!(!b.commit(&first, 99));
    assert!(a.snapshot(&other).is_none());
    assert!(matches!(a.admit(Id { tenant: 0, ..route(105, 14) }, 1), Admission::Invalid));
    assert!(matches!(a.admit(route(105, 99), 1), Admission::Stale));
    assert!(a.commit(&first, 111));
    let second = a.dispatch().unwrap();
    assert_eq!(second.value(), 12);
    assert!(a.commit(&second, 112));
    assert_eq!(a.snapshot(&t1).unwrap().result, Some(111));
    assert_eq!(a.snapshot(&queued).unwrap().result, Some(112));
    assert!(a.conservation() && b.conservation());
}
#[test]
fn four_concurrent_producers_cannot_corrupt_128_unique_results() {
    let mail = Arc::new(Mailbox::new(11, 13, 14, 256).unwrap());
    let mut producers = vec![];
    for p in 0..4 {
        let m = mail.clone();
        producers.push(std::thread::spawn(move || {
            let mut sent = vec![];
            for i in 0..32 {
                let id = 3000 + p * 1000 + i;
                let value = (p * 1000 + i + 1) as i64;
                sent.push((value, ticket(&m, route(id, 14), value)));
            }
            sent
        }));
    }
    let all = producers.into_iter().flat_map(|h| h.join().unwrap()).collect::<Vec<_>>();
    assert_eq!(all.len(), 128);
    for _ in 0..128 {
        let delivery = mail.dispatch().expect("mail missing");
        assert!(mail.commit(&delivery, delivery.value() * 3));
    }
    for (input, t) in all {
        let s = mail.snapshot(&t).unwrap();
        assert_eq!(s.state, TicketState::Processed);
        assert_eq!(s.result, Some(input * 3));
    }
    assert!(mail.conservation());
}
#[test]
fn panic_abandons_pending_and_restart_fences_incarnation() {
    let m = Mailbox::new(11, 13, 14, 2).unwrap();
    let failed = ticket(&m, route(201, 14), -1);
    let turn = m.dispatch().unwrap();
    let abandoned = ticket(&m, route(202, 14), 20);
    assert!(m.panic(&turn));
    assert_eq!(m.snapshot(&failed).unwrap().state, TicketState::Failed);
    assert_eq!(m.snapshot(&abandoned).unwrap().state, TicketState::Abandoned);
    assert!(!m.commit(&turn, 999));
    assert!(matches!(m.admit(route(203, 14), 1), Admission::Closed));
    assert!(m.dispatch().is_none());
    assert!(m.conservation());
    let next = Mailbox::new(11, 13, 15, 1).unwrap();
    assert!(matches!(next.admit(route(204, 14), 1), Admission::Stale));
    assert!(next.snapshot(&failed).is_none());
    let fresh = ticket(&next, route(205, 15), 8);
    let turn = next.dispatch().unwrap();
    assert!(next.commit(&turn, 50));
    assert_eq!(next.snapshot(&fresh).unwrap().result, Some(50));
    assert!(next.conservation());
}
#[test]
fn close_denies_new_receipts_but_drain_can_commit() {
    let m = Mailbox::new(11, 13, 14, 1).unwrap();
    let old = ticket(&m, route(300, 14), 3);
    m.close();
    assert!(matches!(m.admit(route(301, 14), 3), Admission::Closed));
    let turn = m.dispatch().unwrap();
    assert!(m.commit(&turn, 4));
    assert_eq!(m.snapshot(&old).unwrap().result, Some(4));
    assert!(m.conservation());
}

#[test]
fn empty_mailbox_rejects_wrong_tenant_without_spending_credit() {
    let m = Mailbox::new(11, 13, 14, 1).unwrap();
    assert!(matches!(m.admit(Id { tenant: 88, ..route(501, 14) }, 7), Admission::Stale));
    assert!(m.dispatch().is_none());
    let t = ticket(&m, route(501, 14), 7);
    let turn = m.dispatch().unwrap();
    assert!(m.commit(&turn, 14));
    assert!(m.snapshot(&t).is_some());
    assert!(m.conservation());
}

//! Trusted HOST-ONLY destination mailbox ticket/turn correlation.
//! Not a compiler guest ABI, signed transport, authorization check, TenantVM
//! security boundary, restart-proof receipt, or second unary reply registry.
//! The gateway validates capabilities and the scheduler invokes dispatch and
//! settlement; identity fields here are correlation data, NOT credentials.
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use crate::proxy_receipt_gate_v1::ReceiptIdentityV1;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TicketState { Admitted, Started, Processed, Failed, Abandoned }
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TicketSnapshot {
    pub route: ReceiptIdentityV1,
    pub state: TicketState,
    pub result: Option<i64>,
}
#[derive(Debug)]
struct TicketRecord { route: ReceiptIdentityV1, state: TicketState, result: Option<i64> }
#[derive(Debug, Clone)]
pub struct HostTicket {
    owner: Arc<()>,
    record: Arc<Mutex<TicketRecord>>,
}
#[derive(Debug)]
pub enum Admission { Admitted(HostTicket), Invalid, Stale, Closed, Full }
#[derive(Debug)]
pub struct HostDelivery { ticket: HostTicket, value: i64 }
impl HostDelivery { pub fn value(&self) -> i64 { self.value } }
struct Inner {
    queue: VecDeque<(i64, HostTicket)>,
    current: Option<HostTicket>,
    closed: bool,
    fenced: bool,
    admitted: usize,
    processed: usize,
    failed: usize,
    abandoned: usize,
}
pub struct HostProxyMailboxTicketsV1 {
    owner: Arc<()>,
    actor: u64,
    incarnation: u64,
    capacity: usize,
    inner: Mutex<Inner>,
}
fn valid(route: ReceiptIdentityV1) -> bool {
    route.protocol_major == 1 && route.tenant != 0 &&
        route.caller_incarnation != 0 && route.target_actor != 0 &&
        route.target_incarnation != 0 && route.association_generation != 0 &&
        route.placement_epoch != 0 && route.message_id != 0 && route.attempt != 0
}
impl HostProxyMailboxTicketsV1 {
    pub fn new(actor: u64, incarnation: u64, capacity: usize) -> Option<Self> {
        if actor == 0 || incarnation == 0 || !(1..=4096).contains(&capacity) { return None; }
        Some(Self { owner: Arc::new(()), actor, incarnation, capacity,
            inner: Mutex::new(Inner { queue: VecDeque::new(), current: None,
                closed: false, fenced: false, admitted: 0, processed: 0,
                failed: 0, abandoned: 0 }) })
    }
    // The mutex linearizes receipt mint and real mailbox enqueue.
    // No ticket is issued on invalid, stale, closed or full admission.
    pub fn admit(&self, route: ReceiptIdentityV1, value: i64) -> Admission {
        if !valid(route) { return Admission::Invalid; }
        if route.target_actor != self.actor ||
            route.target_incarnation != self.incarnation { return Admission::Stale; }
        let mut inner = self.inner.lock().expect("host mailbox poisoned");
        if inner.closed || inner.fenced { return Admission::Closed; }
        if inner.queue.len() >= self.capacity { return Admission::Full; }
        let ticket = HostTicket { owner: self.owner.clone(),
            record: Arc::new(Mutex::new(TicketRecord {
                route, state: TicketState::Admitted, result: None })) };
        inner.queue.push_back((value, ticket.clone()));
        inner.admitted = inner.admitted.checked_add(1).expect("host mailbox count overflow");
        Admission::Admitted(ticket)
    }
    pub fn snapshot(&self, ticket: &HostTicket) -> Option<TicketSnapshot> {
        if !Arc::ptr_eq(&ticket.owner, &self.owner) { return None; }
        let t = ticket.record.lock().expect("host ticket poisoned");
        Some(TicketSnapshot { route: t.route, state: t.state, result: t.result })
    }
    // Called by trusted carrier when dequeue actually occurs.
    pub fn dispatch(&self) -> Option<HostDelivery> {
        let mut inner = self.inner.lock().expect("host mailbox poisoned");
        if inner.fenced || inner.current.is_some() { return None; }
        let (value, ticket) = inner.queue.pop_front()?;
        {
            let mut record = ticket.record.lock().expect("host ticket poisoned");
            assert_eq!(record.state, TicketState::Admitted);
            record.state = TicketState::Started;
        }
        inner.current = Some(ticket.clone());
        Some(HostDelivery { ticket, value })
    }
    fn active(inner: &Inner, delivery: &HostDelivery) -> bool {
        inner.current.as_ref().is_some_and(|current| {
            Arc::ptr_eq(&current.record, &delivery.ticket.record) &&
                Arc::ptr_eq(&current.owner, &delivery.ticket.owner)
        })
    }
    // Only the scheduler calls this AFTER real handler success; the result
    // belongs to this one mail, not any actor-wide completion counter.
    pub fn commit(&self, delivery: &HostDelivery, value: i64) -> bool {
        let mut inner = self.inner.lock().expect("host mailbox poisoned");
        if inner.fenced || !Self::active(&inner, delivery) { return false; }
        {
            let mut t = delivery.ticket.record.lock().expect("host ticket poisoned");
            if t.state != TicketState::Started { return false; }
            t.state = TicketState::Processed;
            t.result = Some(value);
        }
        inner.current = None;
        inner.processed += 1;
        true
    }
    // The real supervisor CONTROL notification remains a separate priority
    // channel. A panicking turn cannot process later queued requests.
    pub fn panic(&self, delivery: &HostDelivery) -> bool {
        let mut inner = self.inner.lock().expect("host mailbox poisoned");
        if inner.fenced || !Self::active(&inner, delivery) { return false; }
        {
            let mut t = delivery.ticket.record.lock().expect("host ticket poisoned");
            if t.state != TicketState::Started { return false; }
            t.state = TicketState::Failed;
        }
        inner.failed += 1;
        inner.current = None;
        inner.fenced = true;
        inner.closed = true;
        while let Some((_, ticket)) = inner.queue.pop_front() {
            let mut t = ticket.record.lock().expect("host ticket poisoned");
            assert_eq!(t.state, TicketState::Admitted);
            t.state = TicketState::Abandoned;
            inner.abandoned += 1;
        }
        true
    }
    pub fn close(&self) { self.inner.lock().expect("host mailbox poisoned").closed = true; }
    pub fn conservation(&self) -> bool {
        let inner = self.inner.lock().expect("host mailbox poisoned");
        inner.admitted == inner.processed + inner.failed + inner.abandoned +
            inner.queue.len() + usize::from(inner.current.is_some())
    }
}

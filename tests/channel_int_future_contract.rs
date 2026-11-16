//! Narrow Channel<int> -> single-use Future<checked read> contract.
//! This is a HOST REFERENCE MODEL, not executable guest Channel<T> lowering.
//! It guards the future ABI before browser/native implementations are admitted.
use std::collections::{BTreeMap, BTreeSet, VecDeque};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ChannelError { Closed, Full, InvalidHandle, AlreadyConsumed, AlreadySettled }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ReadOutcome { Item(i64), Eof, Cancelled }

struct ChannelInt {
    cap: usize,
    queued: VecDeque<i64>,
    pending: VecDeque<u64>,
    settled: BTreeMap<u64, ReadOutcome>,
    consumed: BTreeSet<u64>,
    next: u64,
    closed: bool,
}
impl ChannelInt {
    fn new(cap: usize) -> Self {
        Self {
            cap, queued: VecDeque::new(), pending: VecDeque::new(),
            settled: BTreeMap::new(), consumed: BTreeSet::new(),
            next: 1, closed: false,
        }
    }
    /// Registration returns immediately: no actor/worker carrier may park.
    fn nb_read(&mut self) -> u64 {
        let ticket=self.next;
        self.next=self.next.checked_add(1).expect("bounded ticket allocation");
        if let Some(item)=self.queued.pop_front() {
            self.settled.insert(ticket,ReadOutcome::Item(item));
        } else if self.closed {
            self.settled.insert(ticket,ReadOutcome::Eof);
        } else {
            self.pending.push_back(ticket);
        }
        ticket
    }
    /// A successful send linearizes at delivery/admission, not consumption.
    fn send(&mut self, item: i64) -> Result<(),ChannelError> {
        if self.closed {return Err(ChannelError::Closed);}
        if let Some(ticket)=self.pending.pop_front() {
            assert!(self.settled.insert(ticket,ReadOutcome::Item(item)).is_none());
        } else if self.queued.len()<self.cap {
            self.queued.push_back(item);
        } else {
            return Err(ChannelError::Full);
        }
        Ok(())
    }
    fn validate(&self,ticket:u64)->Result<(),ChannelError> {
        if ticket==0 || ticket>=self.next {return Err(ChannelError::InvalidHandle);}
        if self.consumed.contains(&ticket){return Err(ChannelError::AlreadyConsumed);}
        Ok(())
    }
    fn peek(&self,ticket:u64)->Result<Option<ReadOutcome>,ChannelError> {
        self.validate(ticket)?;
        Ok(self.settled.get(&ticket).copied())
    }
    /// An await consumes exactly one settled Future, never a second time.
    fn consume(&mut self,ticket:u64)->Result<Option<ReadOutcome>,ChannelError> {
        self.validate(ticket)?;
        let Some(value)=self.settled.remove(&ticket) else {return Ok(None)};
        assert!(self.consumed.insert(ticket));
        Ok(Some(value))
    }
    /// Cancellation of a still-pending future retires its read admission.
    /// An already committed value cannot be erased by cancellation.
    fn cancel(&mut self,ticket:u64)->Result<(),ChannelError> {
        self.validate(ticket)?;
        if self.settled.contains_key(&ticket){return Err(ChannelError::AlreadySettled);}
        let Some(index)=self.pending.iter().position(|&id|id==ticket)
            else {return Err(ChannelError::InvalidHandle)};
        self.pending.remove(index);
        self.settled.insert(ticket,ReadOutcome::Cancelled);
        Ok(())
    }
    /// Already buffered data stays readable. Pending reads complete EOF.
    fn close(&mut self) {
        if self.closed{return;}
        self.closed=true;
        while let Some(ticket)=self.pending.pop_front() {
            assert!(self.settled.insert(ticket,ReadOutcome::Eof).is_none());
        }
    }
}

#[test]
fn pending_registration_is_nonparking_and_resumes_fifo_exactly_once() {
    let mut ch=ChannelInt::new(2);
    let a=ch.nb_read();let b=ch.nb_read();
    assert_eq!(ch.peek(a),Ok(None));
    assert_eq!(ch.peek(b),Ok(None));
    ch.send(10).unwrap();ch.send(20).unwrap();
    assert_eq!(ch.consume(a),Ok(Some(ReadOutcome::Item(10))));
    assert_eq!(ch.consume(b),Ok(Some(ReadOutcome::Item(20))));
    assert_eq!(ch.consume(a),Err(ChannelError::AlreadyConsumed));
    assert!(ch.pending.is_empty());
}

#[test]
fn buffer_capacity_is_bounded_and_no_value_is_speculatively_lost() {
    let mut ch=ChannelInt::new(1);
    ch.send(41).unwrap();
    assert_eq!(ch.send(99),Err(ChannelError::Full));
    let a=ch.nb_read();
    assert_eq!(ch.consume(a),Ok(Some(ReadOutcome::Item(41))));
    ch.send(42).unwrap();
    let b=ch.nb_read();
    assert_eq!(ch.consume(b),Ok(Some(ReadOutcome::Item(42))));
}

#[test]
fn zero_capacity_requires_a_pending_reader_for_rendezvous() {
    let mut ch=ChannelInt::new(0);
    assert_eq!(ch.send(12),Err(ChannelError::Full));
    let a=ch.nb_read();
    ch.send(12).unwrap();
    assert_eq!(ch.consume(a),Ok(Some(ReadOutcome::Item(12))));
}

#[test]
fn cancellation_preserves_next_reader_and_returns_checked_cancel() {
    let mut ch=ChannelInt::new(1);
    let abandoned=ch.nb_read();let next=ch.nb_read();
    ch.cancel(abandoned).unwrap();
    ch.send(42).unwrap();
    assert_eq!(ch.consume(abandoned),Ok(Some(ReadOutcome::Cancelled)));
    assert_eq!(ch.consume(next),Ok(Some(ReadOutcome::Item(42))));
    assert_eq!(ch.cancel(abandoned),Err(ChannelError::AlreadyConsumed));
}

#[test]
fn close_resolves_pending_reads_but_never_drops_queued_payloads() {
    let mut ch=ChannelInt::new(2);
    ch.send(20).unwrap();
    ch.close();ch.close();
    let first=ch.nb_read();let eof=ch.nb_read();
    assert_eq!(ch.consume(first),Ok(Some(ReadOutcome::Item(20))));
    assert_eq!(ch.consume(eof),Ok(Some(ReadOutcome::Eof)));
    assert_eq!(ch.send(1),Err(ChannelError::Closed));
}

#[test]
fn close_with_pending_requests_delivers_eof_without_fake_zero_message() {
    let mut ch=ChannelInt::new(1);
    let a=ch.nb_read();let b=ch.nb_read();
    ch.close();
    assert_eq!(ch.consume(a),Ok(Some(ReadOutcome::Eof)));
    assert_eq!(ch.consume(b),Ok(Some(ReadOutcome::Eof)));
}

#[test]
fn committed_delivery_wins_over_later_cancellation() {
    let mut ch=ChannelInt::new(1);
    let t=ch.nb_read();
    ch.send(123).unwrap();
    assert_eq!(ch.cancel(t),Err(ChannelError::AlreadySettled));
    assert_eq!(ch.consume(t),Ok(Some(ReadOutcome::Item(123))));
}

#[test]
fn malformed_ticket_never_forges_owned_read_result() {
    let mut ch=ChannelInt::new(1);
    assert_eq!(ch.peek(0),Err(ChannelError::InvalidHandle));
    assert_eq!(ch.consume(u64::MAX),Err(ChannelError::InvalidHandle));
    let t=ch.nb_read();
    assert_eq!(ch.peek(t),Ok(None));
}

#[test]
fn guest_read_syntax_cannot_yet_emit_fake_channel_wasm() {
    let source="pub susp fnc main(): int { val pending = nb readch inbox; return await pending; }";
    let d=atto_compiler::compile(source).expect_err(
        "real Channel<int> and Future<Result<Option<int>,Error>> guest ABI is not implemented");
    assert!(matches!(d.code,"E_UNSUPPORTED"|"E_SYNTAX"|"E_UNBOUND"),
        "must fail closed, got {d}");
}

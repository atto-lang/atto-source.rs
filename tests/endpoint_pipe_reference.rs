//! Pure Rust reference oracle for candidate AttoLang endpoint piping.
//! This deliberately does not claim guest stdin, Wasm, or scheduler execution.
use std::collections::VecDeque;

#[derive(Clone, Debug, PartialEq, Eq)]
struct PipeIr { source: &'static str, sink: &'static str }
fn lower(keyword: &str, lhs: &'static str, rhs: &'static str) -> Result<PipeIr, &'static str> {
    if keyword != "pipe" { return Err("E_IO_SYNTAX"); }
    Ok(PipeIr { source: lhs, sink: rhs })
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum FailureKind { Read, Write, Quota, Cancelled }
#[derive(Clone, Debug, PartialEq, Eq)]
struct PipeFailure {
    kind: FailureKind,
    admitted: usize,
    // The one item read but not committed when a write fails/cancels.
    uncommitted: Option<String>,
}

#[derive(Default)]
struct Reader {
    items: VecDeque<String>,
    read_calls: usize,
    fail_read_at: Option<usize>,
    close_calls: usize,
}
impl Reader {
    fn from_items(items: &[&str]) -> Self {
        Self { items: items.iter().map(|s| (*s).to_owned()).collect(), ..Self::default() }
    }
    fn read(&mut self) -> Result<Option<String>, FailureKind> {
        self.read_calls += 1;
        if self.fail_read_at == Some(self.read_calls) { return Err(FailureKind::Read); }
        Ok(self.items.pop_front())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Admission { Ready, Backpressure, Closed, Quota, Cancel }
struct Writer {
    schedule: VecDeque<Admission>,
    output: Vec<String>,
    attempts: usize,
    close_calls: usize,
}
impl Writer {
    fn with_schedule(schedule: &[Admission]) -> Self {
        Self { schedule: schedule.iter().copied().collect(), output: vec![], attempts: 0, close_calls: 0 }
    }
    fn admit(&mut self, item: String) -> Result<(), (Admission, String)> {
        self.attempts += 1;
        match self.schedule.pop_front().unwrap_or(Admission::Ready) {
            Admission::Ready => { self.output.push(item); Ok(()) }
            status => Err((status, item)),
        }
    }
}

// One item at a time. A failed admission returns the owned item to the caller.
fn pump(source: &mut Reader, sink: &mut Writer) -> Result<usize, PipeFailure> {
    // The direct oracle is built on the cooperative state machine below.
    // A bounded tick budget prevents a permanently non-ready sink from
    // creating an infinite busy-loop in a test or CI runner.
    match run_cooperative(source, sink, 1_000_000) {
        PollStep::Finished(n) => Ok(n),
        PollStep::Failed(error) => Err(error),
        PollStep::Pending(_) | PollStep::Progress(_) => unreachable!(),
    }
}

#[test]
fn only_pipe_keyword_normalizes_a_continuous_connection() {
    let pipeline = lower("pipe", "stdio.in", "stdio.out").unwrap();
    assert_eq!(pipeline.source, "stdio.in");
    assert_eq!(pipeline.sink, "stdio.out");
    for old in [">>", "<<"] {
        assert_eq!(lower(old, "stdio.in", "stdio.out"), Err("E_IO_SYNTAX"));
    }
}

#[test]
fn pipe_keyword_relay_preserves_multiple_elements_in_order() {
    let ir = lower("pipe", "stdio.in", "stdio.out").unwrap();
    assert_eq!((ir.source, ir.sink), ("stdio.in", "stdio.out"));
    let mut src = Reader::from_items(&["hello", "", "world"]);
    let mut dst = Writer::with_schedule(&[]);
    assert_eq!(pump(&mut src, &mut dst), Ok(3));
    assert_eq!(dst.output, ["hello", "", "world"]);
    assert_eq!(src.read_calls, 4); // three elements plus one EOF
    assert_eq!(src.close_calls, 0);
    assert_eq!(dst.close_calls, 0);
}

#[test]
fn empty_source_returns_zero_and_leaves_output_open() {
    let mut src = Reader::from_items(&[]);
    let mut dst = Writer::with_schedule(&[]);
    assert_eq!(pump(&mut src, &mut dst), Ok(0));
    assert!(dst.output.is_empty());
    assert_eq!(dst.close_calls, 0);
}

#[test]
fn backpressure_retries_exact_item_without_read_ahead() {
    let mut src = Reader::from_items(&["one", "two"]);
    let mut dst = Writer::with_schedule(&[
        Admission::Backpressure, Admission::Backpressure, Admission::Ready,
        Admission::Backpressure, Admission::Ready,
    ]);
    assert_eq!(pump(&mut src, &mut dst), Ok(2));
    assert_eq!(dst.output, ["one", "two"]);
    assert_eq!(src.read_calls, 3); // exactly two items and EOF
    assert_eq!(dst.attempts, 5);
}

#[test]
fn write_failure_preserves_uncommitted_owned_payload() {
    let mut src = Reader::from_items(&["ok", "recover-me", "not-read"]);
    let mut dst = Writer::with_schedule(&[Admission::Ready, Admission::Closed]);
    assert_eq!(pump(&mut src, &mut dst), Err(PipeFailure {
        kind: FailureKind::Write, admitted: 1, uncommitted: Some("recover-me".into()),
    }));
    assert_eq!(dst.output, ["ok"]);
    assert_eq!(src.items.into_iter().collect::<Vec<_>>(), ["not-read"]);
}

#[test]
fn quota_failure_cannot_drop_or_double_commit() {
    let mut src = Reader::from_items(&["too-big"]);
    let mut dst = Writer::with_schedule(&[Admission::Quota]);
    assert_eq!(pump(&mut src, &mut dst), Err(PipeFailure {
        kind: FailureKind::Quota, admitted: 0, uncommitted: Some("too-big".into()),
    }));
    assert!(dst.output.is_empty());
}

#[test]
fn cancellation_retains_pending_item_after_backpressure() {
    let mut src = Reader::from_items(&["pending", "future"]);
    let mut dst = Writer::with_schedule(&[Admission::Backpressure, Admission::Cancel]);
    assert_eq!(pump(&mut src, &mut dst), Err(PipeFailure {
        kind: FailureKind::Cancelled, admitted: 0, uncommitted: Some("pending".into()),
    }));
    assert_eq!(src.items.into_iter().collect::<Vec<_>>(), ["future"]);
    assert!(dst.output.is_empty());
}

#[test]
fn read_errors_do_not_fabricate_output_or_eof_success() {
    let mut src = Reader::from_items(&["first", "second"]);
    src.fail_read_at = Some(2);
    let mut dst = Writer::with_schedule(&[]);
    assert_eq!(pump(&mut src, &mut dst), Err(PipeFailure {
        kind: FailureKind::Read, admitted: 1, uncommitted: None,
    }));
    assert_eq!(dst.output, ["first"]);
    assert_eq!(src.items.into_iter().collect::<Vec<_>>(), ["second"]);
}


// Cooperative reference model: each poll attempts at most one admission.
// Backpressure yields Pending instead of busy-spinning or reading ahead.
#[derive(Clone, Debug, PartialEq, Eq)]
enum PollStep {
    Progress(usize),
    Pending(String),
    Finished(usize),
    Failed(PipeFailure),
}
#[derive(Default)]
struct CooperativePump {
    admitted: usize,
    pending: Option<String>,
    finished: bool,
    failed: Option<PipeFailure>,
}
impl CooperativePump {
    fn poll(&mut self, source: &mut Reader, sink: &mut Writer) -> PollStep {
        if let Some(failure) = &self.failed { return PollStep::Failed(failure.clone()); }
        if self.finished { return PollStep::Finished(self.admitted); }
        if self.pending.is_none() {
            match source.read() {
                Ok(Some(item)) => self.pending = Some(item),
                Ok(None) => {
                    self.finished = true;
                    return PollStep::Finished(self.admitted);
                }
                Err(kind) => {
                    let failure = PipeFailure { kind, admitted: self.admitted, uncommitted: None };
                    self.failed = Some(failure.clone());
                    return PollStep::Failed(failure);
                }
            }
        }
        let item = self.pending.take().expect("only one pending source item");
        match sink.admit(item) {
            Ok(()) => {
                self.admitted += 1;
                PollStep::Progress(self.admitted)
            }
            Err((Admission::Backpressure, owned)) => {
                self.pending = Some(owned.clone());
                PollStep::Pending(owned)
            }
            Err((state, owned)) => {
                let kind = match state {
                    Admission::Closed => FailureKind::Write,
                    Admission::Quota => FailureKind::Quota,
                    Admission::Cancel => FailureKind::Cancelled,
                    _ => unreachable!(),
                };
                let failure = PipeFailure {
                    kind, admitted: self.admitted, uncommitted: Some(owned),
                };
                self.failed = Some(failure.clone());
                PollStep::Failed(failure)
            }
        }
    }
}
fn run_cooperative(src: &mut Reader, dst: &mut Writer, budget: usize) -> PollStep {
    let mut p = CooperativePump::default();
    for _ in 0..budget {
        match p.poll(src, dst) {
            PollStep::Finished(n) => return PollStep::Finished(n),
            PollStep::Failed(e) => return PollStep::Failed(e),
            PollStep::Progress(_) | PollStep::Pending(_) => {},
        }
    }
    panic!("reference pump did not terminate within {budget} polls");
}

#[test]
fn checked_pipe_keyword_preserves_unicode_empty_messages_and_order() {
    assert_eq!(lower("pipe", "stdio.in", "stdio.out").unwrap().source, "stdio.in");
    {
        let mut src = Reader::from_items(&["", "café", "🦀", "line\n", "\0"]);
        let mut dst = Writer::with_schedule(&[
            Admission::Backpressure, Admission::Ready,
            Admission::Ready, Admission::Backpressure, Admission::Ready,
        ]);
        assert_eq!(run_cooperative(&mut src, &mut dst, 16), PollStep::Finished(5));
        assert_eq!(dst.output, ["", "café", "🦀", "line\n", "\0"]);
        assert_eq!(src.read_calls, 6);
        assert_eq!(src.close_calls + dst.close_calls, 0);
    }
}

#[test]
fn blocked_sink_yields_no_reads_ahead_and_no_spinning() {
    let mut src = Reader::from_items(&["held", "not-yet"]);
    let mut dst = Writer::with_schedule(&vec![Admission::Backpressure; 512]);
    let mut p = CooperativePump::default();
    for attempt in 1..=512 {
        assert_eq!(p.poll(&mut src, &mut dst), PollStep::Pending("held".into()));
        assert_eq!(dst.attempts, attempt);
        assert_eq!(src.read_calls, 1);
        assert!(dst.output.is_empty());
    }
    assert_eq!(src.items.front().map(String::as_str), Some("not-yet"));
    assert_eq!(p.poll(&mut src, &mut dst), PollStep::Progress(1));
    assert_eq!(dst.output, ["held"]);
}

#[test]
fn terminal_eof_is_idempotent_and_does_not_close_stdio() {
    let mut src = Reader::from_items(&[]);
    let mut dst = Writer::with_schedule(&[]);
    let mut p = CooperativePump::default();
    for _ in 0..30 { assert_eq!(p.poll(&mut src, &mut dst), PollStep::Finished(0)); }
    assert_eq!(src.read_calls, 1);
    assert_eq!(src.close_calls + dst.close_calls, 0);
    assert!(dst.output.is_empty());
}

#[test]
fn cancellation_after_pending_returns_owned_item_once() {
    let mut src = Reader::from_items(&["pending", "later"]);
    let mut dst = Writer::with_schedule(&[Admission::Backpressure, Admission::Cancel]);
    let mut p = CooperativePump::default();
    assert_eq!(p.poll(&mut src, &mut dst), PollStep::Pending("pending".into()));
    let expected = PollStep::Failed(PipeFailure {
        kind: FailureKind::Cancelled, admitted: 0,
        uncommitted: Some("pending".into()),
    });
    assert_eq!(p.poll(&mut src, &mut dst), expected);
    assert_eq!(p.poll(&mut src, &mut dst), expected);
    assert_eq!(src.read_calls, 1);
    assert_eq!(dst.attempts, 2);
    assert_eq!(src.items.front().map(String::as_str), Some("later"));
}

#[test]
fn quota_after_partial_progress_keeps_uncommitted_payload() {
    let mut src = Reader::from_items(&["accepted", "uncommitted", "future"]);
    let mut dst = Writer::with_schedule(&[Admission::Ready, Admission::Quota]);
    let mut p = CooperativePump::default();
    assert_eq!(p.poll(&mut src, &mut dst), PollStep::Progress(1));
    assert_eq!(p.poll(&mut src, &mut dst), PollStep::Failed(PipeFailure {
        kind: FailureKind::Quota, admitted: 1,
        uncommitted: Some("uncommitted".into()),
    }));
    assert_eq!(dst.output, ["accepted"]);
    assert_eq!(src.items.front().map(String::as_str), Some("future"));
}

#[test]
fn first_read_error_is_not_eof() {
    let mut src = Reader::from_items(&["unread"]);
    src.fail_read_at = Some(1);
    let mut dst = Writer::with_schedule(&[]);
    assert_eq!(run_cooperative(&mut src, &mut dst, 2), PollStep::Failed(PipeFailure {
        kind: FailureKind::Read, admitted: 0, uncommitted: None,
    }));
    assert_eq!(src.items.front().map(String::as_str), Some("unread"));
    assert!(dst.output.is_empty());
}

#[test]
fn very_large_message_keeps_boundaries_and_all_bytes() {
    let long = format!("{}\0{}", "🦀".repeat(16000), "x".repeat(65536));
    let mut src = Reader::from_items(&[&long]);
    let mut dst = Writer::with_schedule(&[
        Admission::Backpressure, Admission::Backpressure, Admission::Ready,
    ]);
    assert_eq!(run_cooperative(&mut src, &mut dst, 8), PollStep::Finished(1));
    assert_eq!(dst.output, [long]);
    assert_eq!(src.read_calls, 2);
}

fn seeded_rand(seed: &mut u32) -> u32 {
    *seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
    *seed
}

#[test]
fn generated_cooperative_traces_preserve_fifo_exactly_once_and_eof() {
    let mut seed = 0xA770_0207u32;
    for case in 0..400 {
        let n = (seeded_rand(&mut seed) % 40) as usize;
        let mut items = Vec::new();
        let mut schedule = Vec::new();
        for i in 0..n {
            items.push(format!("case={case}/item={i}/seed={}", seeded_rand(&mut seed)));
            let backpressure_polls = seeded_rand(&mut seed) % 5;
            for _ in 0..backpressure_polls { schedule.push(Admission::Backpressure); }
            schedule.push(Admission::Ready);
        }
        let refs = items.iter().map(String::as_str).collect::<Vec<_>>();
        let mut src = Reader::from_items(&refs);
        let mut dst = Writer::with_schedule(&schedule);
        assert_eq!(run_cooperative(&mut src, &mut dst, n * 6 + 2),
            PollStep::Finished(n), "trace {case}");
        assert_eq!(dst.output, items, "lost, reordered or duplicate trace {case}");
        assert_eq!(src.read_calls, n + 1, "excess read on trace {case}");
        assert_eq!(src.close_calls + dst.close_calls, 0);
    }
}

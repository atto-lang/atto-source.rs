//! Pure Rust reference oracle for candidate AttoLang endpoint piping.
//! This deliberately does not claim guest stdin, Wasm, or scheduler execution.
use std::collections::VecDeque;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Spelling { SourceRightSink, SinkLeftSource }

#[derive(Clone, Debug, PartialEq, Eq)]
struct PipeIr { source: &'static str, sink: &'static str }
fn lower(spelling: Spelling, lhs: &'static str, rhs: &'static str) -> PipeIr {
    match spelling {
        Spelling::SourceRightSink => PipeIr { source: lhs, sink: rhs },
        Spelling::SinkLeftSource => PipeIr { source: rhs, sink: lhs },
    }
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
    let mut admitted = 0;
    loop {
        let next = source.read().map_err(|kind| PipeFailure { kind, admitted, uncommitted: None })?;
        let Some(mut item) = next else { return Ok(admitted); }; // EOF, not a message.
        loop {
            match sink.admit(item) {
                Ok(()) => { admitted += 1; break; }
                Err((Admission::Backpressure, still_owned)) => {
                    // No source.read() until pending sink admission resolves.
                    item = still_owned;
                }
                Err((status, still_owned)) => {
                    let kind = match status {
                        Admission::Closed => FailureKind::Write,
                        Admission::Quota => FailureKind::Quota,
                        Admission::Cancel => FailureKind::Cancelled,
                        _ => unreachable!(),
                    };
                    return Err(PipeFailure { kind, admitted, uncommitted: Some(still_owned) });
                }
            }
        }
    }
}

#[test]
fn normalized_direction_is_identical_in_both_spellings() {
    let forward = lower(Spelling::SourceRightSink, "stdio.in", "stdio.out");
    let reverse = lower(Spelling::SinkLeftSource, "stdio.out", "stdio.in");
    assert_eq!(forward, reverse);
    assert_eq!(forward.source, "stdio.in");
    assert_eq!(forward.sink, "stdio.out");
}

#[test]
fn both_orientations_relay_multiple_elements_in_order() {
    for spelling in [Spelling::SourceRightSink, Spelling::SinkLeftSource] {
        let ir = match spelling {
            Spelling::SourceRightSink => lower(spelling, "stdio.in", "stdio.out"),
            Spelling::SinkLeftSource => lower(spelling, "stdio.out", "stdio.in"),
        };
        assert_eq!((ir.source, ir.sink), ("stdio.in", "stdio.out"));
        let mut src = Reader::from_items(&["hello", "", "world"]);
        let mut dst = Writer::with_schedule(&[]);
        assert_eq!(pump(&mut src, &mut dst), Ok(3));
        assert_eq!(dst.output, ["hello", "", "world"]);
        assert_eq!(src.read_calls, 4); // three elements plus one EOF
        assert_eq!(src.close_calls, 0);
        assert_eq!(dst.close_calls, 0);
    }
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

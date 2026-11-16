// Executable Node.js reference model for endpoint I/O pumps.
// NOT compiler-built AttoLang guest stdin / stdout execution.
import test from 'node:test';
import assert from 'node:assert/strict';

const source = (name = 'stdio.in', type = 'string') =>
  ({ name, readable: true, type });
const sink = (name = 'stdio.out', type = 'string') =>
  ({ name, writable: true, type });

function normalized(keyword, from, to) {
  if (keyword !== 'pipe') throw Error('E_IO_SYNTAX');
  if (!from.readable || !to.writable) throw Error('E_IO_CAPABILITY');
  if (from.type !== to.type) throw Error('E_IO_TYPE');
  return { kind: 'PipeConnect', source: from.name, sink: to.name, type: from.type };
}

class Reader {
  constructor(values, failAt = null) {
    this.values = values.slice();
    this.failAt = failAt;
    this.reads = 0;
    this.closed = 0;
  }
  read() {
    if (++this.reads === this.failAt) return { error: 'read' };
    return this.values.length ? { value: this.values.shift() } : { eof: true };
  }
}
class Writer {
  constructor(states = []) {
    this.states = states.slice();
    this.values = [];
    this.attempts = 0;
    this.closed = 0;
  }
  admit(value) {
    ++this.attempts;
    const state = this.states.shift() || 'ready';
    if (state === 'ready') { this.values.push(value); return { ok: true }; }
    if (state === 'blocked') return { pending: true };
    if (['closed', 'quota', 'cancelled'].includes(state)) return { error: state };
    throw Error('invalid test admission');
  }
}
class Pump {
  constructor(reader, writer) {
    this.reader = reader;
    this.writer = writer;
    this.item = undefined;
    this.admitted = 0;
    this.terminal = null;
  }
  tick() {
    if (this.terminal) return this.terminal;
    if (this.item === undefined) {
      const read = this.reader.read();
      if (read.error) return this.terminal =
        { state: 'failed', cause: read.error, admitted: this.admitted, uncommitted: null };
      if (read.eof) return this.terminal =
        { state: 'done', admitted: this.admitted };
      this.item = { value: read.value };
    }
    const write = this.writer.admit(this.item.value);
    if (write.pending) return { state: 'pending', admitted: this.admitted };
    if (write.error) return this.terminal =
      { state: 'failed', cause: write.error, admitted: this.admitted, uncommitted: this.item.value };
    ++this.admitted;
    this.item = undefined;
    return { state: 'progress', admitted: this.admitted };
  }
}
function run(pump, budget = 100000) {
  for (let i = 0; i < budget; ++i) {
    const result = pump.tick();
    if (result.state === 'done' || result.state === 'failed') return result;
  }
  throw Error('test pump exceeded poll budget');
}
function prng(seed) {
  let state = seed >>> 0;
  return () => (state = (Math.imul(state, 1664525) + 1013904223) >>> 0);
}

test('pipe keyword normalizes exactly one producer to consumer connection', () => {
  assert.deepEqual(normalized('pipe', source(), sink()),
    { kind: 'PipeConnect', source: 'stdio.in', sink: 'stdio.out', type: 'string' });
});
test('directional operators are never implicit continuous connections', () => {
  for (const op of ['>>', '<<']) {
    assert.throws(() => normalized(op, source(), sink()), /E_IO_SYNTAX/);
    assert.throws(() => normalized(op, sink(), source()), /E_IO_SYNTAX/);
  }
  assert.throws(() => normalized('pipe', sink(), source()), /E_IO_CAPABILITY/);
  assert.throws(() => normalized('pipe', source('a', 'string'), sink('b', 'int')), /E_IO_TYPE/);
});
test('keyword pipe preserves empty message, Unicode, ordering and EOF', () => {
  const result = normalized('pipe', source(), sink());
  assert.equal(result.source, 'stdio.in');
  const reader = new Reader(['', 'hello', '🦀 café', 'bye']);
  const writer = new Writer();
  assert.deepEqual(run(new Pump(reader, writer)), { state: 'done', admitted: 4 });
  assert.deepEqual(writer.values, ['', 'hello', '🦀 café', 'bye']);
  assert.equal(reader.reads, 5);
  assert.equal(reader.closed + writer.closed, 0);
});
test('empty EOF and subsequent polls never consume or close anything', () => {
  const reader = new Reader([]), writer = new Writer();
  const pump = new Pump(reader, writer);
  for (let i = 0; i < 20; ++i) {
    assert.deepEqual(pump.tick(), { state: 'done', admitted: 0 });
  }
  assert.equal(reader.reads, 1);
  assert.equal(reader.closed + writer.closed, 0);
});
test('backpressure yields; no read ahead, busy spin or duplicate messages', () => {
  const reader = new Reader(['held', 'later']);
  const writer = new Writer(Array(300).fill('blocked'));
  const pump = new Pump(reader, writer);
  for (let n = 1; n <= 300; ++n) {
    assert.equal(pump.tick().state, 'pending');
    assert.equal(reader.reads, 1);
    assert.equal(writer.attempts, n);
    assert.deepEqual(writer.values, []);
  }
  assert.equal(pump.tick().state, 'progress');
  assert.deepEqual(writer.values, ['held']);
  assert.deepEqual(reader.values, ['later']);
});
test('cancellation returns uncommitted item without any speculative read', () => {
  const reader = new Reader(['inflight', 'later']);
  const writer = new Writer(['blocked', 'cancelled']);
  const pump = new Pump(reader, writer);
  assert.equal(pump.tick().state, 'pending');
  assert.deepEqual(pump.tick(), {
    state: 'failed', cause: 'cancelled', admitted: 0, uncommitted: 'inflight'
  });
  assert.deepEqual(reader.values, ['later']);
  assert.equal(reader.reads, 1);
  assert.equal(writer.attempts, 2);
});
test('write quota and close preserve partial progress and owned payload', () => {
  for (const reason of ['quota', 'closed']) {
    const reader = new Reader(['committed', 'retry', 'later']);
    const writer = new Writer(['ready', reason]);
    assert.deepEqual(run(new Pump(reader, writer)), {
      state: 'failed', cause: reason, admitted: 1, uncommitted: 'retry'
    });
    assert.deepEqual(writer.values, ['committed']);
    assert.deepEqual(reader.values, ['later']);
  }
});
test('read error is not EOF or a fabricated data message', () => {
  const reader = new Reader(['one', 'unread'], 2), writer = new Writer();
  assert.deepEqual(run(new Pump(reader, writer)), {
    state: 'failed', cause: 'read', admitted: 1, uncommitted: null
  });
  assert.deepEqual(writer.values, ['one']);
  assert.deepEqual(reader.values, ['unread']);
});
test('large Unicode + NUL message remains one original element', () => {
  const content = '🦀'.repeat(16000) + '\0' + 'x'.repeat(65536);
  const reader = new Reader([content]), writer = new Writer(['blocked', 'ready']);
  assert.deepEqual(run(new Pump(reader, writer)), { state: 'done', admitted: 1 });
  assert.deepEqual(writer.values, [content]);
});
test('750 seeded FIFO, exactly-once and EOF conformance traces', () => {
  const next = prng(0xA7700207);
  for (let c = 0; c < 750; ++c) {
    const count = next() % 35;
    const messages = Array.from({ length: count },
      (_, n) => 'trace-' + c + '-message-' + n + '-' + next());
    const readiness = [];
    for (let n = 0; n < count; ++n) {
      for (let b = next() % 5; b > 0; --b) readiness.push('blocked');
      readiness.push('ready');
    }
    const reader = new Reader(messages), writer = new Writer(readiness);
    assert.deepEqual(run(new Pump(reader, writer)), {
      state: 'done', admitted: count
    }, 'trace ' + c);
    assert.deepEqual(writer.values, messages, 'trace ' + c);
    assert.equal(reader.reads, count + 1, 'trace ' + c);
    assert.equal(reader.closed + writer.closed, 0);
  }
});

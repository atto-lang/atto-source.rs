// Source-only test oracle for AttoLang's proposed actor/stream/channel `pipe`.
// This is NOT compiled Atto guest execution or a browser ABI implementation.
import assert from 'node:assert/strict';
import test from 'node:test';

const pending = () => {
  let resolve, reject;
  const promise = new Promise((ok, no) => { resolve = ok; reject = no; });
  return { promise, resolve, reject };
};
const cancelled = () => Object.assign(new Error('cancelled'), { code: 'CANCELLED' });

class BoundedSink {
  constructor(capacity) {
    if (!Number.isSafeInteger(capacity) || capacity < 0) throw new TypeError('capacity');
    this.capacity = capacity;
    this.buffer = [];
    this.readers = [];
    this.writers = [];
    this.externalClosed = false;
    this.maxBuffered = 0;
  }
  async write(value, signal) {
    if (signal?.aborted) throw cancelled();
    if (this.externalClosed) throw new Error('sink closed');
    const reader = this.readers.shift();
    if (reader) { reader.resolve({ done: false, value }); return; }
    if (this.buffer.length < this.capacity) {
      this.buffer.push(value);
      this.maxBuffered = Math.max(this.maxBuffered, this.buffer.length);
      return;
    }
    const waiter = pending();
    const entry = { value, ...waiter };
    this.writers.push(entry);
    const abort = () => {
      const index = this.writers.indexOf(entry);
      if (index >= 0) { this.writers.splice(index, 1); entry.reject(cancelled()); }
    };
    signal?.addEventListener('abort', abort, { once: true });
    try { await waiter.promise; }
    finally { signal?.removeEventListener('abort', abort); }
  }
  async read() {
    if (this.buffer.length) {
      const value = this.buffer.shift();
      const writer = this.writers.shift();
      if (writer) {
        this.buffer.push(writer.value);
        this.maxBuffered = Math.max(this.maxBuffered, this.buffer.length);
        writer.resolve();
      }
      return { done: false, value };
    }
    const writer = this.writers.shift();
    if (writer) { writer.resolve(); return { done: false, value: writer.value }; }
    if (this.externalClosed) return { done: true };
    const waiter = pending();
    this.readers.push(waiter);
    return waiter.promise;
  }
  closeExternal() {
    this.externalClosed = true;
    for (const writer of this.writers.splice(0)) writer.reject(new Error('sink closed'));
    if (!this.buffer.length) for (const reader of this.readers.splice(0)) reader.resolve({ done: true });
  }
}

function connect(source, sink) {
  if (!source || typeof source[Symbol.asyncIterator] !== 'function') throw new TypeError('PipeSource');
  if (!sink || typeof sink.write !== 'function') throw new TypeError('PipeSink');
  const abort = new AbortController();
  const stats = { admitted: 0, state: 'running', error: null };
  const done = (async () => {
    try {
      for await (const value of source) {
        if (abort.signal.aborted) throw cancelled();
        await sink.write(value, abort.signal);
        stats.admitted++;
      }
      stats.state = 'completed';
    } catch (error) {
      stats.state = error.code === 'CANCELLED' ? 'cancelled' : 'failed';
      stats.error = error.message;
    }
    return { ...stats };
  })();
  return { get_done: () => done, cancel: () => abort.abort(), stats };
}
const producer = async function* (values) { for (const value of values) yield value; };

// Exactly one read and one write are represented by read/write calls; `pipe`
// alone runs a sequence. The channel is externally owned and must not close.
test('pipe preserves zero/one/many ordered messages without closing borrowed sink', async () => {
  for (const values of [[], [42], [1, 2, 3, 4]]) {
    const sink = new BoundedSink(values.length || 1);
    const connection = connect(producer(values), sink);
    const done = await connection.get_done();
    assert.equal(done.state, 'completed');
    assert.equal(done.admitted, values.length);
    assert.equal(sink.externalClosed, false);
    const received = [];
    for (const _ of values) received.push((await sink.read()).value);
    assert.deepEqual(received, values);
    assert.ok(sink.maxBuffered <= sink.capacity);
  }
});

test('capacity zero is rendezvous and never buffers an admitted message', async () => {
  const sink = new BoundedSink(0);
  const connection = connect(producer([41, 42]), sink);
  await Promise.resolve(); await Promise.resolve();
  assert.equal(connection.stats.admitted, 0);
  assert.equal(sink.buffer.length, 0);
  assert.deepEqual((await sink.read()).value, 41);
  assert.deepEqual((await sink.read()).value, 42);
  assert.equal((await connection.get_done()).admitted, 2);
  assert.equal(sink.maxBuffered, 0);
});

test('bounded capacity backpressures a producer until a consumer reads', async () => {
  const sink = new BoundedSink(1);
  const connection = connect(producer([1, 2, 3]), sink);
  // A short microtask turn is enough for first write, not a timing assertion.
  for (let i = 0; i < 10; i++) await Promise.resolve();
  assert.equal(connection.stats.admitted, 1);
  assert.equal(sink.buffer.length, 1);
  assert.equal((await sink.read()).value, 1);
  assert.equal((await sink.read()).value, 2);
  assert.equal((await sink.read()).value, 3);
  assert.equal((await connection.get_done()).state, 'completed');
  assert.equal(sink.maxBuffered, 1);
});

test('cancel blocked write never later delivers its uncommitted value', async () => {
  const sink = new BoundedSink(1);
  const connection = connect(producer([10, 20]), sink);
  for (let i = 0; i < 10; i++) await Promise.resolve();
  assert.equal(connection.stats.admitted, 1);
  connection.cancel();
  const done = await connection.get_done();
  assert.equal(done.state, 'cancelled');
  assert.equal(done.admitted, 1);
  assert.equal((await sink.read()).value, 10);
  assert.equal(sink.buffer.length, 0);
  assert.equal(sink.writers.length, 0);
});

test('failure preserves partial admitted count and does not fabricate EOF', async () => {
  const sink = new BoundedSink(2);
  async function* failAfterFirst() { yield 7; throw new Error('producer failed'); }
  const done = await connect(failAfterFirst(), sink).get_done();
  assert.deepEqual({ state: done.state, admitted: done.admitted, error: done.error },
    { state: 'failed', admitted: 1, error: 'producer failed' });
  assert.equal((await sink.read()).value, 7);
  assert.equal(sink.externalClosed, false);
});

test('invalid source/sink contracts fail before creating background work', () => {
  const sink = new BoundedSink(1);
  assert.throws(() => connect(42, sink), /PipeSource/);
  assert.throws(() => connect(producer([1]), {}), /PipeSink/);
  assert.equal(sink.writers.length, 0);
});

test('actor ordinary sends and supervisor CONTROL do not enter emit data path', async () => {
  const sink = new BoundedSink(1);
  const applicationSends = [];
  const supervisorControls = [];
  const actor = {
    send: value => applicationSends.push(value),
    control: value => supervisorControls.push(value),
    emit_values: () => producer([42]),
  };
  actor.send(5);
  actor.control('PANIC');
  const done = await connect(actor.emit_values(), sink).get_done();
  assert.equal(done.admitted, 1);
  assert.equal((await sink.read()).value, 42);
  assert.deepEqual(applicationSends, [5]);
  assert.deepEqual(supervisorControls, ['PANIC']);
});

test('stream-to-actor transform allows zero-to-many emitted values', async () => {
  const sink = new BoundedSink(3);
  async function* transform(input) {
    for await (const value of input) {
      if (value === 1) { yield 10; yield 11; }
      if (value === 3) yield 30;
    }
  }
  const done = await connect(transform(producer([1, 2, 3])), sink).get_done();
  assert.equal(done.admitted, 3);
  assert.deepEqual([(await sink.read()).value, (await sink.read()).value,
    (await sink.read()).value], [10, 11, 30]);
});

test('a channel can bridge generator source to an actor sink without unbounded reads', async () => {
  const middle = new BoundedSink(1);
  const output = new BoundedSink(2);
  const up = connect(producer([41, 42]), middle);
  const fromChannel = async function* () {
    yield (await middle.read()).value;
    yield (await middle.read()).value;
  };
  const down = connect(fromChannel(), output);
  const results = await Promise.all([up.get_done(), down.get_done()]);
  assert.deepEqual(results.map(x => x.admitted), [2, 2]);
  assert.deepEqual([(await output.read()).value, (await output.read()).value], [41, 42]);
  assert.equal(middle.maxBuffered, 1);
  assert.equal(middle.externalClosed, false);
});

test('two consumers competing for one channel never receive implicit broadcast', async () => {
  const sink = new BoundedSink(2);
  const done = await connect(producer([10, 20]), sink).get_done();
  const [a, b] = await Promise.all([sink.read(), sink.read()]);
  assert.equal(done.admitted, 2);
  assert.deepEqual([a.value, b.value].sort((x, y) => x - y), [10, 20]);
  assert.equal(sink.buffer.length, 0);
});

test('stream readable view cannot masquerade as writable pipe sink', () => {
  const readable = producer([1, 2]);
  assert.throws(() => connect(producer([3]), readable), /PipeSink/);
});

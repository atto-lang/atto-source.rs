// Trusted Worker/Node host-only process stdio adapter (NOT an Atto guest ABI).
// All ports must be explicitly injected by the host. No global process, stdin,
// stdout, fs, or inherited descriptor is read from this module.
//
// Channel-like reads return Result<Option<string>,IoError> as {ok,value} or
// {ok:false,error:{kind}}. Writes return Result<void,IoError>. The source-compiled
// Atto Channel<string> type and checked Wasm representation do NOT exist yet.
const ok = value => ({ok:true,value});
const err = kind => ({ok:false,error:{kind}});
const eof = () => ok({some:false});
const some = value => ok({some:true,value});
const uint = (n,min,max) => Number.isSafeInteger(n) && n >= min && n <= max;
const utf8 = new TextDecoder('utf-8',{fatal:true});
const encoder = new TextEncoder();

class InputPort {
  #iterator;
  #record = [];
  #queue = [];
  #queuedBytes = 0;
  #waiting = null;
  #pulling = false;
  #busy = false;
  #streamActive = false;
  #terminal = null;
  #done = false;
  #revoked = false;
  #limits;

  constructor(source,limits) {
    if (!source || typeof source[Symbol.asyncIterator] !== 'function')
      throw new TypeError('explicit async-iterable stdin source required');
    this.#iterator = source[Symbol.asyncIterator]();
    if (!this.#iterator || typeof this.#iterator.next !== 'function')
      throw new TypeError('invalid stdin iterator');
    this.#limits = limits;
    Object.freeze(this);
  }

  async readch({signal}={}) {
    if (this.#revoked) return err('Revoked');
    if (this.#busy || this.#streamActive) return err('ReaderBusy');
    this.#busy = true;
    try {return await this.#read(signal)}
    finally {this.#busy = false}
  }

  // The lease is claimed at stream() creation, not by independently calling
  // readch on the same port. Closing the iterator does not close host stdin.
  stream() {
    if (this.#revoked) throw new Error('Revoked');
    if (this.#busy || this.#streamActive) throw new Error('ReaderBusy');
    this.#streamActive = true;
    let active = true, reading = false;
    const release = () => {
      active = false;
      this.#streamActive = false;
      this.#cancelPending('Cancelled');
    };
    const iterator = {
      async next() {
        if (!active) return {done:true};
        if (reading) throw new Error('ReaderBusy');
        reading = true;
        try {
          const answer = await thisPort.#read();
          if (!answer.ok) {
            release();
            throw new Error(answer.error.kind);
          }
          if (!answer.value.some) {release();return {done:true}}
          return {done:false,value:answer.value.value};
        } finally {reading = false}
      },
      async return() {release();return {done:true}},
      [Symbol.asyncIterator]() {return this},
    };
    const thisPort = this;
    return iterator;
  }

  revoke() {
    if (this.#revoked) return;
    this.#revoked = true;
    this.#queue = [];
    this.#record = [];
    this.#queuedBytes = 0;
    this.#cancelPending('Revoked');
    // Do not call iterator.return(): injected stdin may be process-owned.
  }

  async #read(signal) {
    if (this.#revoked) return err('Revoked');
    if (signal?.aborted) return err('Cancelled');
    if (this.#queue.length) return this.#dequeue();
    if (this.#terminal) return err(this.#terminal);
    if (this.#done) return eof();
    return new Promise(resolve => {
      const wait = {resolve,signal,abort:null};
      if (signal) {
        wait.abort = () => {
          if (this.#waiting === wait) this.#cancelPending('Cancelled');
        };
        signal.addEventListener('abort',wait.abort,{once:true});
        if (signal.aborted) {
          signal.removeEventListener('abort',wait.abort);
          resolve(err('Cancelled'));
          return;
        }
      }
      this.#waiting = wait;
      void this.#pump();
    });
  }

  #dequeue() {
    const item = this.#queue.shift();
    this.#queuedBytes -= item.bytes;
    return some(item.text);
  }

  #settle(answer) {
    const wait = this.#waiting;
    if (!wait) return;
    this.#waiting = null;
    if (wait.abort) wait.signal.removeEventListener('abort',wait.abort);
    wait.resolve(answer);
  }
  #cancelPending(reason) {
    if (this.#waiting) this.#settle(err(reason));
  }
  #wake() {
    if (!this.#waiting) return;
    if (this.#revoked) this.#settle(err('Revoked'));
    else if (this.#queue.length) this.#settle(this.#dequeue());
    else if (this.#terminal) this.#settle(err(this.#terminal));
    else if (this.#done) this.#settle(eof());
  }
  #fail(kind) {this.#terminal = kind;this.#wake()}
  #finishLine(trimCr) {
    const length = this.#record.length;
    const size = length && trimCr && this.#record[length-1] === 13 ? length-1 : length;
    let decoded;
    try {decoded = utf8.decode(Uint8Array.from(this.#record.slice(0,size)))}
    catch {this.#record=[];this.#fail('MalformedUtf8');return false}
    this.#record = [];
    if (this.#queue.length >= this.#limits.maxQueuedRecords ||
        size > this.#limits.maxQueuedBytes - this.#queuedBytes) {
      this.#fail('Quota');return false;
    }
    this.#queue.push({text:decoded,bytes:size});
    this.#queuedBytes += size;
    return true;
  }

  async #pump() {
    if (this.#pulling) return;
    this.#pulling = true;
    let emptyChunks = 0;
    try {
      while (this.#waiting && !this.#queue.length && !this.#terminal &&
             !this.#done && !this.#revoked) {
        let next;
        try {next = await this.#iterator.next()}
        catch {this.#fail('SourceFailure');break}
        if (this.#revoked) break;
        if (!next || typeof next.done !== 'boolean') {this.#fail('InvalidInput');break}
        if (next.done) {
          if (this.#record.length && !this.#finishLine(false)) break;
          this.#done = true;
          this.#wake();
          break;
        }
        const bytes = next.value;
        if (!(bytes instanceof Uint8Array)) {this.#fail('InvalidInput');break}
        if (bytes.byteLength > this.#limits.maxChunkBytes) {this.#fail('Quota');break}
        if (!bytes.byteLength && ++emptyChunks > 16) {this.#fail('Quota');break}
        if (bytes.byteLength) emptyChunks = 0;
        for (const byte of bytes) {
          if (byte === 10) {
            if (!this.#finishLine(true)) break;
          } else {
            if (this.#record.length >= this.#limits.maxRecordBytes) {
              this.#fail('RecordTooLarge');break;
            }
            this.#record.push(byte);
          }
        }
        this.#wake();
      }
    } finally {
      this.#pulling = false;
      // A cancelling reader may be replaced while an old pending input.next()
      // was awaiting readiness. Resume only when the new owner needs data.
      if (this.#waiting && !this.#queue.length && !this.#terminal &&
          !this.#done && !this.#revoked) void this.#pump();
    }
  }
}

class OutputPort {
  #sink;
  #queue = [];
  #bytes = 0;
  #draining = false;
  #revoked = false;
  #failed = false;
  #limits;
  constructor(sink,limits) {
    if (typeof sink !== 'function') throw new TypeError('explicit async output writer required');
    this.#sink = sink;
    this.#limits = limits;
    Object.freeze(this);
  }
  writech(value) {
    if (this.#revoked) return Promise.resolve(err('Revoked'));
    if (this.#failed) return Promise.resolve(err('SinkFailure'));
    if (typeof value !== 'string') return Promise.resolve(err('InvalidValue'));
    const bytes = encoder.encode(value);
    if (bytes.length > this.#limits.maxWriteBytes) return Promise.resolve(err('Quota'));
    if (this.#queue.length >= this.#limits.maxPendingWrites ||
        bytes.length > this.#limits.maxPendingBytes-this.#bytes)
      return Promise.resolve(err('Full'));
    return new Promise(resolve => {
      this.#queue.push({bytes,resolve});
      this.#bytes += bytes.length;
      void this.#drain();
    });
  }
  revoke() {
    this.#revoked = true;
    // Do not claim a transport has failed if its current write was already
    // admitted. Pending (not yet started) writes must be rejected promptly.
    if (this.#draining) {
      for (const job of this.#queue.slice(1)) job.resolve(err('Revoked'));
      this.#queue = this.#queue.slice(0,1);
    } else {
      for (const job of this.#queue) job.resolve(err('Revoked'));
      this.#queue = [];
    }
  }
  async #drain() {
    if (this.#draining) return;
    this.#draining = true;
    try {
      while (this.#queue.length) {
        const job = this.#queue[0];
        if (this.#revoked) {job.resolve(err('Revoked'))}
        else {
          try {await this.#sink(new Uint8Array(job.bytes));job.resolve(ok(undefined))}
          catch {this.#failed = true;job.resolve(err('SinkFailure'))}
        }
        this.#bytes -= job.bytes.length;
        this.#queue.shift();
        if (this.#failed || this.#revoked) {
          for (const pending of this.#queue) pending.resolve(err(this.#revoked?'Revoked':'SinkFailure'));
          this.#queue = [];
          this.#bytes = 0;
        }
      }
    } finally {this.#draining = false}
  }
}

// Trusted host constructor. An untrusted guest must never be given this JS
// object, native descriptors, async iterators, or callback functions. A future
// guest ABI must gate each import on tenant identity/generation.
export class StdioTextHostV1 {
  constructor({stdin,stdout,stderr,tenantId,generation,maxRecordBytes=65536,
    maxChunkBytes=262144,maxQueuedRecords=128,maxQueuedBytes=262144,
    maxWriteBytes=65536,maxPendingWrites=64,maxPendingBytes=262144}={}) {
    if (!uint(generation,1,0x7fffffff)) throw new TypeError('tenant/process generation required');
    if (typeof tenantId !== 'string' || !tenantId.trim() || tenantId.length>128)
      throw new TypeError('explicit trusted tenant identity required');
    for (const n of [maxRecordBytes,maxChunkBytes,maxQueuedRecords,maxQueuedBytes,
      maxWriteBytes,maxPendingWrites,maxPendingBytes]) {
      if (!uint(n,1,1048576)) throw new TypeError('invalid stdio quota');
    }
    const limits = {maxRecordBytes,maxChunkBytes,maxQueuedRecords,maxQueuedBytes,
      maxWriteBytes,maxPendingWrites,maxPendingBytes};
    const input = new InputPort(stdin,limits);
    const out = new OutputPort(stdout,limits);
    const error = new OutputPort(stderr,limits);
    this.tenantId = tenantId;
    this.generation = generation;
    this.stdio = Object.freeze({in:input,out,err:error});
    this.process = Object.freeze({stdin:input,stdout:out,stderr:error});
    this.revoke = () => {input.revoke();out.revoke();error.revoke()};
    Object.freeze(this);
  }
}

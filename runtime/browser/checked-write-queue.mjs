// Narrow browser Worker host-side admission for a future AttoLang nb/do writech ABI.
// Host-only: this does NOT make nb/do source statements executable.
// Single event-loop domain. No timers, threads, unbounded queues, or lost errors.
// The sink adapter MUST be non-parking and returns checked {ok, written} or {ok:false, error}.
const encoder = new TextEncoder();

const error = (code, message) => ({code, message});
const failed = (code, message) => ({ok:false, error:error(code,message)});
function resultFromSink(raw, expected) {
  if (raw?.ok === false && typeof raw.error?.code === 'string') {
    return failed(raw.error.code, String(raw.error.message ?? raw.error.code));
  }
  if (raw?.ok === true && Number.isSafeInteger(raw.written) && raw.written === expected) {
    return {ok:true, written:expected};
  }
  return failed('E_IO_ADAPTER','adapter did not acknowledge the entire write');
}
function asBytes(input) {
  if (typeof input === 'string') return encoder.encode(input);
  if (input instanceof Uint8Array) return input;
  throw new TypeError('payload must be UTF-8 string or Uint8Array');
}
function ownerKey(owner,generation) { return owner + '\0' + generation; }

export class CheckedWriteQueue {
  constructor({slots=8, maxBytes=65536, sink, supervisor=null}={}) {
    if (!Number.isSafeInteger(slots) || slots<1 || slots>1024 ||
        !Number.isSafeInteger(maxBytes) || maxBytes<1 || maxBytes>64*1024*1024 ||
        typeof sink !== 'function' ||
        (supervisor!==null && typeof supervisor!=='function')) {
      throw new TypeError('bounded slots/maxBytes, an asynchronous sink and optional supervisor are required');
    }
    this.slots=slots;this.maxBytes=maxBytes;this.sink=sink;this.supervisor=supervisor;
    this.closed=false;this._next=1;this._entries=new Map();this._fifo=[];
    this._usedBytes=0;this._running=null;this._scheduled=false;
    this._stopped=new Set();this._supervisorFailures=[];
  }

  get stats() {
    return Object.freeze({pending:this._entries.size, bytes:this._usedBytes,
      queued:this._fifo.filter(x=>x.state==='queued').length,
      committed:this._running!==null, closed:this.closed,
      supervisorFailures:this._supervisorFailures.length});
  }

  // Submit is synchronous and returns promptly. On rejection, the ORIGINAL
  // payload remains owned by the caller. On acceptance the runtime copies
  // the bytes, preventing caller mutation or use-after-scope corruption.
  submitWrite({owner,generation=0,payload,mode='nb',onOutcome=null,onSuccess=null}={}) {
    if(typeof owner!=='string'||!owner||owner.length>128||
       !Number.isSafeInteger(generation)||generation<0||
       !['nb','do'].includes(mode) ||
       (onOutcome!==null&&typeof onOutcome!=='function') ||
       (onSuccess!==null&&typeof onSuccess!=='function') ||
       (onOutcome!==null&&onSuccess!==null) ||
       (mode==='nb'&&(onOutcome!==null||onSuccess!==null))) {
      return {ok:false,...failed('E_IO_SYNTAX','invalid owner/mode/callback contract'),payload};
    }
    if(mode==='do'&&!this.supervisor) {
      return {ok:false,...failed('E_IO_SUPERVISOR','detached writes require a supervisor error path'),payload};
    }
    if(this.closed) return {...failed('E_IO_CLOSED','writer is closed'),payload};
    if(this._stopped.has(ownerKey(owner,generation))) {
      return {...failed('E_IO_OWNER','owner generation already stopped'),payload};
    }
    const incoming=asBytes(payload);
    if(incoming.byteLength>this.maxBytes ||
       this._entries.size>=this.slots ||
       this._usedBytes+incoming.byteLength>this.maxBytes) {
      return {...failed('E_IO_QUOTA','pending write capacity exhausted'),payload};
    }
    if(this._next>Number.MAX_SAFE_INTEGER) return {...failed('E_IO_QUOTA','ticket id exhausted'),payload};
    const id=this._next++;
    let resolve;
    const future=new Promise(ok=>{resolve=ok;});
    const entry={id,owner,generation,bytes:incoming.slice(),size:incoming.byteLength,
      mode,onOutcome,onSuccess,resolve,state:'queued'};
    this._entries.set(id,entry);this._fifo.push(entry);this._usedBytes+=entry.size;
    this._schedule();
    return mode==='nb'?{ok:true,ticket:id,future}:{ok:true,ticket:id};
  }

  // Only queued work is cancellable. An adapter write that already committed
  // cannot be rolled back. This explicit commit boundary prevents ghost writes
  // after a successful pre-commit cancellation.
  cancel(ticket) {
    const x=this._entries.get(ticket);
    if(!x||x.state!=='queued') return false;
    this._settle(x,failed('E_IO_CANCELLED','write cancelled before commit'));
    return true;
  }
  stopOwner(owner,generation=0) {
    const key=ownerKey(owner,generation);this._stopped.add(key);
    for(const x of [...this._entries.values()]) {
      if(x.owner===owner&&x.generation===generation&&x.state==='queued') {
        this._settle(x,failed('E_IO_CANCELLED','owner stopped before commit'));
      }
    }
    // A committed adapter write may finish, but its callback is fenced.
  }
  close() {
    if(this.closed)return;
    this.closed=true;
    for(const x of [...this._entries.values()]) {
      if(x.state==='queued') this._settle(x,failed('E_IO_CLOSED','writer closed before commit'));
    }
  }
  _schedule() {
    if(this._scheduled||this._running!==null) return;
    this._scheduled=true;
    queueMicrotask(()=>{this._scheduled=false;this._pump();});
  }
  _pump() {
    if(this._running!==null)return;
    let x;
    while((x=this._fifo.shift())) {
      if(x.state!=='queued')continue;
      if(this.closed||this._stopped.has(ownerKey(x.owner,x.generation))) {
        this._settle(x,failed('E_IO_CANCELLED','owner or writer no longer active'));
        continue;
      }
      x.state='committed';this._running=x;
      // The queue never performs a synchronous blocking system call; the
      // adapter is invoked in a microtask. Actual I/O readiness is adapter-owned.
      Promise.resolve().then(()=>this.sink(x.bytes.slice(),
        {ticket:x.id,owner:x.owner,generation:x.generation}))
        .then(result=>this._settle(x,resultFromSink(result,x.size)),
          err=>this._settle(x,failed('E_IO_ADAPTER',String(err?.message??err))))
        .finally(()=>{this._running=null;this._schedule();});
      break;
    }
  }
  _supervise(x,outcome) {
    try { this.supervisor?.({owner:x.owner,generation:x.generation,ticket:x.id,outcome}); }
    catch(err) {
      // Fatal supervisor failures must remain observable, never disguised as
      // successful I/O. Host integrations MUST check this after teardown.
      this._supervisorFailures.push(err);
    }
  }
  _settle(x,outcome) {
    if(x.state==='settled')return;
    x.state='settled';
    this._entries.delete(x.id);
    this._usedBytes-=x.size;
    x.resolve(outcome);
    if(x.mode==='do') {
      const stopped=this._stopped.has(ownerKey(x.owner,x.generation));
      if(stopped) {
        if(!outcome.ok)this._supervise(x,outcome);
      } else {
        try {
          if(x.onOutcome) x.onOutcome(outcome);
          else if(outcome.ok) x.onSuccess?.();
          else this._supervise(x,outcome);
        } catch(err) {
          this._supervise(x,failed('E_IO_CALLBACK',String(err?.message??err)));
        }
      }
    }
    // Releasing a cancelled queued ticket may make capacity immediately
    // available; an already committed write still runs to completion.
  }
  takeSupervisorFailures() {
    const failures=this._supervisorFailures.slice();
    this._supervisorFailures.length=0;
    return failures;
  }
}

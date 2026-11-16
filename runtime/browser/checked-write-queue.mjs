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
// Generations are monotone per owner; a stopped generation also fences every
// older generation. Owner history stays bounded for the lifetime of the queue.
function validOwner(owner,generation) {
  return typeof owner==='string' && owner.length>0 && owner.length<=128 &&
    !owner.includes('\0') && Number.isSafeInteger(generation) && generation>=0;
}
function errorMessage(value) {
  try { return String(value?.message ?? value); }
  catch { return 'unreadable adapter or callback error'; }
}

export class CheckedWriteQueue {
  constructor({slots=8, maxBytes=65536, maxOwners=1024, sink, supervisor=null}={}) {
    if (!Number.isSafeInteger(slots) || slots<1 || slots>1024 ||
        !Number.isSafeInteger(maxOwners) || maxOwners<1 || maxOwners>65536 ||
        !Number.isSafeInteger(maxBytes) || maxBytes<1 || maxBytes>64*1024*1024 ||
        typeof sink !== 'function' ||
        (supervisor!==null && typeof supervisor!=='function')) {
      throw new TypeError('bounded slots/maxBytes, an asynchronous sink and optional supervisor are required');
    }
    this.slots=slots;this.maxBytes=maxBytes;this.maxOwners=maxOwners;
    this.sink=sink;this.supervisor=supervisor;
    this.closed=false;this._next=1;this._entries=new Map();this._fifo=[];
    this._usedBytes=0;this._running=null;this._scheduled=false;
    this._owners=new Map();this._supervisorFailures=[];this._fatalSupervisor=false;
  }

  get stats() {
    return Object.freeze({pending:this._entries.size, bytes:this._usedBytes,
      queued:this._fifo.filter(x=>x.state==='queued').length,
      committed:this._running?.state==='committed', closed:this.closed,
      owners:this._owners.size, fatalSupervisor:this._fatalSupervisor,
      supervisorFailures:this._supervisorFailures.length});
  }

  // Submit is synchronous and returns promptly. On rejection, the ORIGINAL
  // payload remains owned by the caller. On acceptance the runtime copies
  // the bytes, preventing caller mutation or use-after-scope corruption.
  submitWrite({owner,generation=0,payload,mode='nb',onOutcome=null,onSuccess=null}={}) {
    if(!validOwner(owner,generation)||
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
    const record=this._owners.get(owner);
    if(record && generation<=record.stoppedThrough) {
      return {...failed('E_IO_OWNER','owner generation already stopped'),payload};
    }
    if(!record && this._owners.size>=this.maxOwners) {
      return {...failed('E_IO_QUOTA','owner registry capacity exhausted'),payload};
    }
    let incoming;
    try { incoming=asBytes(payload).slice(); }
    catch(err) {
      return {...failed('E_IO_PAYLOAD',errorMessage(err)),payload};
    }
    if(incoming.byteLength>this.maxBytes ||
       this._entries.size>=this.slots ||
       this._usedBytes+incoming.byteLength>this.maxBytes) {
      return {...failed('E_IO_QUOTA','pending write capacity exhausted'),payload};
    }
    if(this._next>Number.MAX_SAFE_INTEGER) return {...failed('E_IO_QUOTA','ticket id exhausted'),payload};
    const id=this._next++;
    let resolve;
    const future=new Promise(ok=>{resolve=ok;});
    const entry={id,owner,generation,bytes:incoming,size:incoming.byteLength,
      mode,onOutcome,onSuccess,resolve,state:'queued'};
    if(!record) this._owners.set(owner,{stoppedThrough:-1});
    this._entries.set(id,entry);this._fifo.push(entry);this._usedBytes+=entry.size;
    this._schedule();
    return mode==='nb'?{ok:true,ticket:id,future}:{ok:true,ticket:id};
  }

  // Only queued work is cancellable. An adapter write that already committed
  // cannot be rolled back. This explicit commit boundary prevents ghost writes
  // after a successful pre-commit cancellation.
  // Cancellation is owner-scoped. A raw sequential ticket is not authority.
  // The caller MUST pass runtime-validated actor owner/generation identity.
  cancel(ticket,identity={}) {
    const owner=identity?.owner,generation=identity?.generation;
    const x=this._entries.get(ticket);
    if(!validOwner(owner,generation)||!x || x.owner!==owner ||
       x.generation!==generation || x.state!=='queued') return false;
    this._settle(x,failed('E_IO_CANCELLED','write cancelled before commit'));
    return true;
  }
  _stoppedOwner(owner,generation) {
    const record=this._owners.get(owner);
    return !!record && generation<=record.stoppedThrough;
  }
  stopOwner(owner,generation=0) {
    if(!validOwner(owner,generation)) return false;
    let record=this._owners.get(owner);
    if(!record) {
      if(this._owners.size>=this.maxOwners||this.closed) return false;
      record={stoppedThrough:-1};
      this._owners.set(owner,record);
    }
    record.stoppedThrough=Math.max(record.stoppedThrough,generation);
    for(const x of [...this._entries.values()]) {
      if(x.owner===owner&&x.generation<=record.stoppedThrough&&x.state==='queued') {
        this._settle(x,failed('E_IO_CANCELLED','owner stopped before commit'));
      }
    }
    // A committed adapter write may finish, but callbacks stay fenced.
    return true;
  }
  close() {
    if(this.closed)return;
    this.closed=true;
    for(const x of [...this._entries.values()]) {
      if(x.state==='queued') this._settle(x,failed('E_IO_CLOSED','writer closed before commit'));
    }
    if(this._entries.size===0) this._owners.clear();
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
      if(this.closed||this._stoppedOwner(x.owner,x.generation)) {
        this._settle(x,failed('E_IO_CANCELLED','owner or writer no longer active'));
        continue;
      }
      this._running=x; // selected, but NOT yet committed to the adapter
      Promise.resolve().then(()=>{
        // A queued operation may have been cancelled/closed after it was
        // selected but before the adapter is actually invoked. Recheck here.
        if(x.state!=='queued')return;
        if(this.closed||this._stoppedOwner(x.owner,x.generation)) {
          this._settle(x,failed('E_IO_CANCELLED',
            'cancelled before actual sink invocation'));
          return;
        }
        x.state='committed';
        return this.sink(x.bytes.slice(),
          {ticket:x.id,owner:x.owner,generation:x.generation});
      }).then(raw=>{
        if(x.state!=='committed')return;
        let outcome;
        try { outcome=resultFromSink(raw,x.size); }
        catch(err) {
          outcome=failed('E_IO_ADAPTER',errorMessage(err));
        }
        this._settle(x,outcome);
      },err=>{
        if(x.state==='committed')
          this._settle(x,failed('E_IO_ADAPTER',errorMessage(err)));
      }).catch(err=>{
        // An adversarial thenable / outcome getter can throw from promise
        // assimilation. Never strand credits or cause an unhandled rejection.
        if(x.state!=='settled')
          this._settle(x,failed('E_IO_ADAPTER',errorMessage(err)));
      }).finally(()=>{
        this._running=null;this._schedule();
      });
      break;
    }
  }
  _recordSupervisorFailure(err) {
    // A permanently failing error handler must not grow the failure ledger
    // indefinitely. Fail closed; outstanding accepted writes <= slot budget.
    this._fatalSupervisor=true;
    if(this._supervisorFailures.length<this.slots)
      this._supervisorFailures.push(err);
    this.close();
  }
  _supervise(x,outcome) {
    try {
      const returned=this.supervisor?.({
        owner:x.owner,generation:x.generation,ticket:x.id,outcome
      });
      if(returned && typeof returned.then==='function') {
        // Supervisor delivery must be synchronous and observable. An async
        // rejection is consumed only to avoid a second unhandled failure.
        Promise.resolve(returned).catch(()=>{});
        this._recordSupervisorFailure(
          new Error('supervisor must not return a pending Promise'));
      }
    } catch(err) {
      this._recordSupervisorFailure(err);
    }
  }
  _settle(x,outcome) {
    if(x.state==='settled')return;
    // Remove queued tombstones immediately. Repeated cancel/re-admit with
    // one stalled committed write cannot grow an unbounded FIFO array.
    if(x.state==='queued') this._fifo=this._fifo.filter(y=>y!==x);
    x.state='settled';
    this._entries.delete(x.id);
    this._usedBytes-=x.size;
    x.resolve(outcome);
    if(x.mode==='do') {
      const stopped=this._stoppedOwner(x.owner,x.generation);
      if(stopped) {
        if(!outcome.ok)this._supervise(x,outcome);
      } else {
        try {
          let returned;
          if(x.onOutcome) returned=x.onOutcome(outcome);
          else if(outcome.ok) returned=x.onSuccess?.();
          else this._supervise(x,outcome);
          // Language callbacks are non-suspending unless explicitly declared.
          // Do not allow an ignored Promise to create an unobserved failure.
          if(returned && typeof returned.then==='function') {
            // The caller attempted to suspend in a non-suspending callback.
            // Report one policy failure, consume any later rejection to
            // prevent a second supervisor notice or unhandled Promise.
            Promise.resolve(returned).catch(()=>{});
            this._supervise(x,failed('E_IO_CALLBACK',
              'detached callback must not return a pending Promise'));
          }
        } catch(err) {
          this._supervise(x,failed('E_IO_CALLBACK',String(err?.message??err)));
        }
      }
    }
    // Close is terminal: release historical owner fences when all accepted
    // tickets finish. Before then they are needed to suppress stale callbacks.
    if(this.closed && this._entries.size===0) this._owners.clear();
    // Releasing a cancelled queued ticket may make capacity immediately
    // available; an already committed write still runs to completion.
  }
  takeSupervisorFailures() {
    const failures=this._supervisorFailures.slice();
    this._supervisorFailures.length=0;
    return failures;
  }
}

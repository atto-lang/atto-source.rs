// M9 bounded serial-actor host for the restricted M8 compiler-emitted guest.
// One dedicated Worker per actor; does not claim pooled scheduling, full
// atto.browser.guest.v2 parity, or hard-real-time CPU preemption.
const invariant=(ok,why)=>{if(!ok)throw new Error(why)};
export class M9SupervisorHost {
  #actors=new Map();
  #nextId=0;
  #maxActors;
  #mailbox;
  #budget;
  #onControl;
  #controls=[];
  #scheduled=false;
  #closed=false;
  constructor({maxActors=16,mailboxCapacity=128,reductionBudget=4,onControl=()=>{}}={}) {
    invariant(Number.isSafeInteger(maxActors)&&maxActors>=1&&maxActors<=128,'actor limit');
    invariant(Number.isSafeInteger(mailboxCapacity)&&mailboxCapacity>=1&&mailboxCapacity<=10000,'mailbox limit');
    invariant(Number.isSafeInteger(reductionBudget)&&reductionBudget>=1&&reductionBudget<=65536,'slice limit');
    invariant(typeof onControl==='function','CONTROL receiver missing');
    this.#maxActors=maxActors;this.#mailbox=mailboxCapacity;
    this.#budget=reductionBudget;this.#onControl=onControl;
  }
  spawn(worker,bytes,supervisor_id=0) {
    invariant(!this.#closed&&this.#actors.size<this.#maxActors,'actor capacity/closed');
    invariant(worker&&typeof worker.postMessage==='function','real Worker required');
    invariant(Number.isSafeInteger(supervisor_id)&&supervisor_id>=0,'supervisor id');
    invariant(bytes instanceof Uint8Array && WebAssembly.validate(bytes),'compiled guest required');
    const actor_id=this.#nextId++;
    const ref=Object.freeze({actor_id,generation:1});
    const actor={ref,worker,supervisor_id,ready:false,accepting:true,
      fenced:false,retired:false,busy:false,phase:'idle',active:null,
      inbox:[],nextMessage:1,accepted:0,completed:0,failed:0,
      abandoned:0,yielded:0,resultSum:0n,lastResult:null,
      pendingControl:null,lastControl:null,controlsDelivered:0,workerStopped:false};
    this.#actors.set(actor_id,actor);
    const receive=m=>this.#receive(actor,m);
    const fatal=e=>this.#workerFatal(actor,100,String(e?.message||e||'worker terminated'));
    if(typeof worker.on==='function') {
      worker.on('message',receive);worker.on('error',fatal);
      worker.on('exit',code=>{if(!actor.workerStopped)fatal('worker exited '+code)});
    } else {
      worker.addEventListener('message',e=>receive(e.data));
      worker.addEventListener('error',fatal);
    }
    worker.postMessage({type:'INIT',actor_id,generation:1,bytes});
    return ref;
  }
  #resolve(ref) {
    if(!ref||!Number.isSafeInteger(ref.actor_id))return null;
    const a=this.#actors.get(ref.actor_id);
    return a&&a.ref===ref&&!a.retired?a:null;
  }
  try_send(ref,amount) {
    const a=this.#resolve(ref);
    if(!a)return 'STALE';
    if(this.#closed||!a.accepting||a.fenced)return 'CLOSED';
    if(a.inbox.length>=this.#mailbox)return 'FULL';
    let v;
    try {v=BigInt(amount)} catch {return 'INVALID'}
    if(v < -(1n<<63n) || v >= 1n<<63n)return 'INVALID';
    a.inbox.push({message_id:a.nextMessage++,amount:String(v)});
    a.accepted++;
    this.#pulse();
    return 'ACCEPTED';
  }
  request_stop(ref) {
    const a=this.#resolve(ref);
    if(!a)return false;
    a.accepting=false;
    this.#control(a,'STOP',0,'graceful stop',a.active?.message_id??0);
    this.#pulse();
    return true;
  }
  request_panic(ref,reason='host-initiated panic') {
    const a=this.#resolve(ref);
    if(!a)return false;
    this.#fence(a,101,String(reason),a.active?.message_id??0);
    return true;
  }
  #control(a,kind,fault_code,reason,message_id) {
    if(a.lastControl==='PANIC')return;
    if(kind==='STOP' && (a.lastControl==='STOP' || a.pendingControl))return;
    if(a.pendingControl) {
      if(kind==='PANIC') {
        a.pendingControl.kind='PANIC';
        a.pendingControl.fault_code=fault_code;
        a.pendingControl.reason=reason;
        a.pendingControl.message_id=message_id;
      }
      return;
    }
    const notice={kind,actor_id:a.ref.actor_id,generation:a.ref.generation,
      supervisor_id:a.supervisor_id,message_id,fault_code,reason};
    a.pendingControl=notice;
    this.#controls.push(a);
    this.#pulse();
  }
  #fence(a,code,reason,id) {
    if(a.fenced)return;
    a.accepting=false;a.fenced=true;
    a.abandoned+=a.inbox.length;a.inbox.length=0;
    this.#control(a,'PANIC',code,reason,id);
    if(a.active&&!a.workerStopped) {
      // The in-flight frame is still conserved until its Worker is quiescent.
      a.phase='cancel';
      a.busy=true;
      a.worker.postMessage({type:'CANCEL',actor_id:a.ref.actor_id,
        generation:a.ref.generation,message_id:a.active.message_id});
    } else {
      this.#retire(a);
    }
    this.#pulse();
  }
  #workerFatal(a,code,reason) {
    if(a.workerStopped)return;
    a.workerStopped=true;
    a.accepting=false;a.fenced=true;
    a.abandoned+=a.inbox.length;a.inbox.length=0;
    const id=a.active?.message_id??0;
    if(a.active){a.failed++;a.active=null;}
    a.busy=false;
    this.#control(a,'PANIC',code,reason,id);
    this.#retire(a);
    this.#pulse();
  }
  #receive(a,msg) {
    if(a.workerStopped)return;
    if(!msg||msg.actor_id!==a.ref.actor_id||msg.generation!==a.ref.generation) {
      this.#workerFatal(a,101,'forged/stale worker identity');
      return;
    }
    if(msg.type==='READY' && !a.ready && !a.active) {
      a.ready=true;this.#pulse();return;
    }
    if(msg.type==='PROTOCOL_ERROR') {
      this.#fence(a,101,'worker protocol violation',a.active?.message_id??0);
      return;
    }
    if(msg.type==='CLOSED' && a.fenced && a.phase==='cancel') {
      if(a.active){a.abandoned++;a.active=null;}
      a.busy=false;this.#retire(a);this.#pulse();return;
    }
    if(!a.active||msg.message_id!==a.active.message_id||!a.busy) {
      this.#fence(a,101,'stale or forged turn sequence',a.active?.message_id??0);
      return;
    }
    if(a.fenced) {
      // A slice reply may precede the queued CANCEL acknowledgment. Never
      // retire/reclaim the accepted frame on a YIELD/DONE race: wait for
      // the Worker's CLOSED event proving physically quiescent cleanup.
      if(msg.type==='DONE'||msg.type==='FAILED'||msg.type==='YIELD') {
        a.phase='cancel';a.busy=true;
      }
      return;
    }
    if(msg.type==='BEGUN' && a.phase==='begin') {
      a.busy=false;a.phase='suspended';this.#pulse();return;
    }
    if(msg.type==='YIELD' && a.phase==='slice' &&
       Number.isSafeInteger(msg.pc) && msg.pc>=0 && msg.pc<=4096) {
      a.busy=false;a.phase='suspended';a.yielded++;this.#pulse();return;
    }
    if(msg.type==='DONE' && a.phase==='slice') {
      let result;
      try{result=BigInt(msg.result)}catch{return this.#fence(a,101,'forged DONE result',msg.message_id)}
      // Result must be within signed-i64, with no completed-turn replays.
      if(result < -(1n<<63n) || result >= (1n<<63n))
        return this.#fence(a,101,'forged out-of-range result',msg.message_id);
      a.completed++;a.lastResult=result;a.resultSum+=result;
      a.busy=false;a.active=null;a.phase='idle';
      this.#retire(a);this.#pulse();return;
    }
    if(msg.type==='FAILED' && a.phase==='slice') {
      a.failed++;a.busy=false;a.active=null;
      const fault=Number.isSafeInteger(msg.fault_code)&&[1,2,101].includes(msg.fault_code)?
        msg.fault_code:101;
      this.#fence(a,fault,fault===1?'checked add overflow':
        fault===2?'checked index overflow':'guest protocol violation',msg.message_id);
      return;
    }
    this.#fence(a,101,'forged continuation response',a.active?.message_id??0);
  }
  #retire(a) {
    if(!a.accepting&&!a.active&&!a.inbox.length)a.retired=true;
  }
  #pulse() {
    if(this.#scheduled)return;
    this.#scheduled=true;
    queueMicrotask(()=>this.#dispatch());
  }
  #dispatch() {
    this.#scheduled=false;
    // Dedicated CONTROL queue is NOT subject to DATA mailbox capacity.
    while(this.#controls.length) {
      const a=this.#controls.shift();
      const c=a.pendingControl;
      if(!c)continue;
      a.pendingControl=null;
      a.lastControl=c.kind;
      a.controlsDelivered++;
      try{this.#onControl(Object.freeze({...c}))}
      catch(e){ /* callbacks cannot corrupt settlement or stall dispatcher */ }
    }
    for(const a of this.#actors.values()) {
      if(!a.ready||a.busy||a.fenced||a.retired||a.workerStopped)continue;
      if(a.active) {
        a.busy=true;a.phase='slice';
        a.worker.postMessage({type:'SLICE',actor_id:a.ref.actor_id,
          generation:a.ref.generation,message_id:a.active.message_id,budget:this.#budget});
      } else if(a.inbox.length) {
        a.active=a.inbox.shift();
        a.busy=true;a.phase='begin';
        a.worker.postMessage({type:'BEGIN',actor_id:a.ref.actor_id,
          generation:a.ref.generation,...a.active});
      } else this.#retire(a);
    }
  }
  snapshot(ref) {
    const a=this.#actors.get(ref?.actor_id);
    invariant(a&&a.ref===ref,'stale actor ref');
    return Object.freeze({actor_id:a.ref.actor_id,generation:a.ref.generation,
      accepted:a.accepted,completed:a.completed,failed:a.failed,
      abandoned:a.abandoned,queued:a.inbox.length,inFlight:a.active?1:0,
      yielded:a.yielded,accepting:a.accepting,retired:a.retired,
      lastControl:a.lastControl,controlsDelivered:a.controlsDelivered,
      lastResult:a.lastResult,resultSum:a.resultSum,workerStopped:a.workerStopped});
  }
  check_conservation(ref) {
    const s=this.snapshot(ref);
    invariant(s.accepted===s.completed+s.failed+s.abandoned+s.queued+s.inFlight,
      'M9 accepted DATA conservation violated: '+JSON.stringify({
        accepted:s.accepted,completed:s.completed,failed:s.failed,
        abandoned:s.abandoned,queued:s.queued,inFlight:s.inFlight}));
    return s;
  }
  async await_terminal(ref,timeoutMs=15000) {
    invariant(Number.isSafeInteger(timeoutMs)&&timeoutMs>=1&&timeoutMs<=60000,'timeout');
    const until=Date.now()+timeoutMs;
    do {
      if(this.snapshot(ref).retired)return true;
      await new Promise(resolve=>setTimeout(resolve,2));
    }while(Date.now()<until);
    return this.snapshot(ref).retired;
  }
  async close() {
    if(this.#closed)return;
    this.#closed=true;
    for(const a of this.#actors.values()) {
      if(!a.retired&&!a.fenced) {
        a.accepting=false;
        this.#control(a,'STOP',0,'host closing',a.active?.message_id??0);
      }
    }
    this.#pulse();
    for(const a of this.#actors.values()) {
      // If a hostile Worker never acknowledges a slice, force physical
      // termination before settling accepted in-flight work.
      if(!a.retired && !(await this.await_terminal(a.ref,3000))) {
        try{await Promise.resolve(a.worker.terminate())}catch{}
        this.#workerFatal(a,100,'worker shutdown deadline');
      }
      a.workerStopped=true;
      try{await Promise.resolve(a.worker.terminate())}catch{}
    }
  }
}

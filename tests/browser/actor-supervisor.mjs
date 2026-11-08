// Stage 4A: host-owned, priority CONTROL channel for actor terminal notices.
// The child Worker never receives a recording capability or authors a notice.
// Four reserved slots match the browser host's four-actor admission limit.
const KINDS = new Set(['completed','failed','cancelled']);
export class SupervisorControl {
  constructor({capacity=4,parent_id=0}={}) {
    if(!Number.isSafeInteger(capacity)||capacity<1||capacity>4||
       !Number.isSafeInteger(parent_id)||parent_id<0){
      throw new Error('E_CONTROL_CONFIG');
    }
    this.capacity=capacity;
    this.parent_id=parent_id;
    this.slots=new Map();
    this.supervisors=new Map();
    this.tokens=new WeakMap();
    this.queue=[];
    this.waiters=[];
  }

  // Only the parent allocates this unguessable identity. Do not send tokens
  // to an actor Worker or accept one from worker.onmessage.
  register_actor(actor_handle,actor_id) {
    if(!Number.isSafeInteger(actor_handle)||actor_handle<1||
       !Number.isSafeInteger(actor_id)||actor_id<0)throw new Error('E_CONTROL_ID');
    if(this.slots.has(actor_handle))throw new Error('E_CONTROL_DUP_HANDLE');
    if(this.slots.size>=this.capacity)throw new Error('E_CONTROL_CAPACITY');
    const token=Object.freeze(Object.create(null));
    this.tokens.set(token,actor_handle);
    this.slots.set(actor_handle,{actor_id,parent_id:this.parent_id,watcher:null,terminal:null});
    return token;
  }

  // Runtime-supervisor handles are parent-owned numeric capabilities created
  // only by the compiler's supervisor_create host import. No child-worker
  // message is permitted to call this method.
  create_supervisor(type_id) {
    if(!Number.isSafeInteger(type_id)||type_id<0||
       this.supervisors.size>=this.capacity)throw new Error('E_CONTROL_SUPERVISOR_LIMIT');
    const handle=this.supervisors.size+1;
    this.supervisors.set(handle,{type_id});
    return handle;
  }

  // Zero means watch admitted; any nonzero status is Err<SupervisionError>.
  // A terminal or previously assigned child can never be adopted again.
  watch_actor(supervisor_handle, actor_handle) {
    if(!Number.isSafeInteger(supervisor_handle)||supervisor_handle<1||
       !Number.isSafeInteger(actor_handle)||actor_handle<1) return 1;
    if(!this.supervisors.has(supervisor_handle))return 1;
    const slot=this.slots.get(actor_handle);
    if(!slot)return 1;
    if(slot.terminal)return 3;
    if(slot.watcher!==null)return 2;
    slot.watcher=supervisor_handle;
    slot.parent_id=supervisor_handle;
    return 0;
  }

  record_terminal(token,state,code,processed) {
    if(!token||typeof token!=='object'||!this.tokens.has(token))
      throw new Error('E_CONTROL_CAPABILITY');
    const actor_handle=this.tokens.get(token);
    const slot=this.slots.get(actor_handle);
    if(slot.terminal)return false; // First terminal transition wins forever.
    if(!KINDS.has(state)||!Number.isSafeInteger(processed)||processed<0||
       (state==='completed'&&code!=='OK')||
       (state!=='completed'&&(typeof code!=='string'||code.length>128||
          !/^E_[A-Z0-9_]+$/.test(code))))throw new Error('E_CONTROL_EVENT');
    const kind=state==='completed'?'terminated':
      state==='cancelled'?'cancelled':
      code.startsWith('E_ACTOR_PANIC_')?'panic':
      code.endsWith('_TIMEOUT')?'timeout':'failure';
    const notice=Object.freeze({
      route:'CONTROL',kind,state,code,
      actor_handle,actor_id:slot.actor_id,parent_id:slot.parent_id,processed,
    });
    slot.terminal=notice; // Durable record BEFORE notifying a waiter.
    const waiter=this.waiters.shift();
    if(waiter)waiter(notice);
    else this.queue.push(notice);
    return notice;
  }

  get_terminal(actor_handle) {
    return this.slots.get(actor_handle)?.terminal??null;
  }

  // CONTROL is always consumed before ordinary actor messages. The ordinary
  // mailbox is supplied by its owner; this channel cannot be saturated by it.
  take_next(mailbox=[]) {
    if(this.queue.length)return {channel:'CONTROL',notice:this.queue.shift()};
    if(!Array.isArray(mailbox))throw new Error('E_CONTROL_MAILBOX');
    return mailbox.length?{channel:'MESSAGE',message:mailbox.shift()}:null;
  }

  take_control() {
    return this.queue.shift()??null;
  }

  wait_control() {
    const ready=this.take_control();
    return ready ? Promise.resolve(ready):
      new Promise(resolve=>this.waiters.push(resolve));
  }

  drain_control() {
    return this.queue.splice(0);
  }
}

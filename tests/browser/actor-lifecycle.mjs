// Private browser-host lifecycle contract; pure and unit-testable without Workers.
// Mailbox admission is synchronous. Completion is terminal and exactly-once,
// never inferred from send() success or from a single receive() reply.
export const TERMINAL = Object.freeze(['FINISHED', 'CLOSED', 'FAILED', 'TERMINATED']);
export class ActorLifecycle {
  constructor(handle, actorId, capacity=2) {
    if(!Number.isInteger(handle)||handle<1||!Number.isInteger(actorId)||actorId<0
      ||!Number.isInteger(capacity)||capacity<1)throw new Error('E_ACTOR_CONFIG');
    this.handle=handle;
    this.actorId=actorId;
    this.capacity=capacity;
    this.state='STARTING';
    this.admissionClosed=false;
    this.queue=[];
    this.inFlight=0;
    this.admitted=0;
    this.delivered=0;
    this.externalDelivered=0;
    this.completion=null;
    this.completionCount=0;
  }
  send(value) {
    if(typeof value!=='bigint'||BigInt.asIntN(64,value)!==value)
      throw new Error('E_MESSAGE_TYPE');
    if(this.admissionClosed||this.completion)return 2; // Err(Closed)
    if(this.queue.length+this.inFlight>=this.capacity)return 1; // Err(Full)
    this.queue.push(value);
    this.admitted++;
    return 0; // Ok(()) is admission only
  }
  close() {
    if(this.admissionClosed||this.completion)return false;
    this.admissionClosed=true;
    return true;
  }
  ready() {
    if(this.state!=='STARTING'||this.completion)throw new Error('E_ACTOR_STATE');
    this.state='READY';
  }
  draining() {
    if(this.state!=='READY'&&this.state!=='DRAINING')
      throw new Error('E_ACTOR_STATE');
    this.state='DRAINING';
  }
  dequeue() {
    if(this.state!=='DRAINING'||this.inFlight!==0)throw new Error('E_ACTOR_STATE');
    if(!this.queue.length)return null;
    this.inFlight=1;
    return this.queue.shift();
  }
  acknowledged() {
    if(this.inFlight!==1||this.state!=='DRAINING')throw new Error('E_ACTOR_STATE');
    this.inFlight=0;
    this.delivered++;
  }
  acknowledgedExternal() {
    if(this.state!=='DRAINING'||this.completion)throw new Error('E_ACTOR_STATE');
    this.externalDelivered++;
  }
  complete(status,reason=null) {
    if(this.completion)return false;
    if(!TERMINAL.includes(status))throw new Error('E_ACTOR_STATE');
    if((status==='FINISHED'||status==='CLOSED')&&(this.queue.length||this.inFlight))
      throw new Error('E_ACTOR_NOT_DRAINED');
    this.state=status;
    this.admissionClosed=true;
    const pending=this.queue.length+this.inFlight;
    this.queue.length=0;
    this.inFlight=0;
    this.completionCount++;
    this.completion=Object.freeze({
      handle:this.handle, actorId:this.actorId, state:status,
      reason:reason===null?null:String(reason),
      admitted:this.admitted, delivered:this.delivered,
      externalDelivered:this.externalDelivered, dropped:pending
    });
    return true;
  }
}

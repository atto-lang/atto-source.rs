// Supervisor CONTROL tests exercise the identical host-owned module used in
// actual Chromium, with deterministic synthetic workers only for races.
import test from 'node:test';
import assert from 'node:assert/strict';
import { SupervisorControl } from './browser/actor-supervisor.mjs';
import { ActorSession, ACTOR_ADMISSION } from './browser/actor-lifecycle.mjs';

const tick=()=>new Promise(resolve=>setImmediate(resolve));
class FakeWorker {
  constructor(options={}) {this.options=options;this.sent=[];this.count=0;this.terminated=0}
  emit(data) {this.onmessage?.({data})}
  postMessage(message) {
    this.sent.push(message);
    if(message.kind==='init') {
      if(!this.options.hold_ready) queueMicrotask(()=>this.emit({kind:'ready'}));
    } else if(message.kind==='message') {
      if(this.options.hold_reply)return;
      if(this.options.panic)queueMicrotask(
        ()=>this.emit({kind:'error',message:'E_ACTOR_PANIC_1'}));
      else {
        this.count++;
        queueMicrotask(()=>this.emit({
          kind:'reply',seq:message.seq,value:message.value+1n,
        }));
      }
    } else if(message.kind==='close'&&!this.options.hold_done) {
      queueMicrotask(()=>this.emit({
        kind:'done',processed:this.count+(this.options.bad_done?1:0),
      }));
    }
  }
  terminate() {this.terminated++}
}
const make=(id=0,options={})=>{
  const supervisor=new SupervisorControl({capacity:4,parent_id:123});
  const worker=new FakeWorker(options);
  const actor=new ActorSession(worker,id,new Uint8Array(),{
    supervisor,actor_handle:1,timeout_ms:options.timeout_ms||1000,
  });
  return {supervisor,worker,actor};
};

test('supervisor constructor and host-owned actor slot validation fail closed',()=>{
  for(const config of [{capacity:0},{capacity:5},{capacity:-1},
    {capacity:1.5},{parent_id:-1},{parent_id:NaN},{parent_id:1.5}]) {
    assert.throws(()=>new SupervisorControl(config),/E_CONTROL_CONFIG/);
  }
  const supervisor=new SupervisorControl({capacity:1});
  for(const [handle,id] of [[0,0],[-1,0],[1,-1],[1,NaN],
    [1,1.5],[NaN,0]]) {
    assert.throws(()=>supervisor.register_actor(handle,id),/E_CONTROL_ID/);
  }
  const token=supervisor.register_actor(1,0);
  assert.ok(token);
  assert.throws(()=>supervisor.register_actor(1,0),/E_CONTROL_DUP_HANDLE/);
  assert.throws(()=>supervisor.register_actor(2,0),/E_CONTROL_CAPACITY/);
  const worker=new FakeWorker();
  assert.throws(()=>new ActorSession(worker,0,new Uint8Array(),{
    supervisor,actor_handle:2,
  }),/E_CONTROL_CAPACITY/);
});

test('forged tokens, cloned tokens and malformed terminal records cannot enter CONTROL',()=>{
  const supervisor=new SupervisorControl();
  const token=supervisor.register_actor(1,7);
  for(const unknown of [null,{},Object.freeze({}),"token",7,[]]) {
    assert.throws(()=>supervisor.record_terminal(unknown,'completed','OK',0),
      /E_CONTROL_CAPABILITY/);
  }
  for(const [state,code,count] of [
    ['success','OK',0],['completed','E_ACTOR_PANIC_1',0],
    ['failed','OK',0],['failed','evil',0],
    ['failed','E_X',-1],['completed','OK',Infinity],
    ['cancelled','E_X',1.5],
  ]) {
    assert.throws(()=>supervisor.record_terminal(token,state,code,count),
      /E_CONTROL_EVENT/);
  }
  assert.equal(supervisor.drain_control().length,0);
  assert.equal(supervisor.get_terminal(1),null);
});

test('real terminal done acknowledgment creates one immutable host-originated CONTROL record',async()=>{
  const {supervisor,worker,actor}=make(12);
  assert.equal(supervisor.get_terminal(1),null);
  assert.equal(actor.admit(41n),ACTOR_ADMISSION.OK);
  const output=await actor.drain_and_close();
  assert.equal(output.processed,1);
  const events=supervisor.drain_control();
  assert.equal(events.length,1);
  const record=events[0];
  assert.deepEqual(record,{
    route:'CONTROL',kind:'terminated',state:'completed',code:'OK',
    actor_handle:1,actor_id:12,parent_id:123,processed:1,
  });
  assert.equal(Object.isFrozen(record),true);
  assert.throws(()=>{record.state='failed'},TypeError);
  assert.strictEqual(supervisor.get_terminal(1),record);
  assert.deepEqual(supervisor.drain_control(),[]);
  worker.emit({kind:'error',message:'E_ACTOR_PANIC_1'});
  actor.cancel();
  assert.strictEqual(supervisor.get_terminal(1),record);
  assert.equal(worker.terminated,1);
});

test('child-originated fake CONTROL message cannot mint supervisor priority event',async()=>{
  const {supervisor,worker,actor}=make(3,{hold_ready:true});
  worker.emit({
    kind:'CONTROL',actor_handle:1,parent_id:123,kind_name:'terminated',
    state:'completed',processed:0,
  });
  await assert.rejects(actor.get_done(),/E_ACTOR_PROTOCOL/);
  const notices=supervisor.drain_control();
  assert.equal(notices.length,1);
  assert.equal(notices[0].kind,'failure');
  assert.equal(notices[0].state,'failed');
  assert.equal(notices[0].code,'E_ACTOR_PROTOCOL');
});

test('CONTROL has priority over a saturated ordinary mailbox',()=>{
  const supervisor=new SupervisorControl();
  const token=supervisor.register_actor(1,4);
  const ordinary=Array.from({length:32},(_,i)=>BigInt(i));
  const notice=supervisor.record_terminal(token,'cancelled','E_ACTOR_CANCELLED',0);
  const first=supervisor.take_next(ordinary);
  assert.equal(first.channel,'CONTROL');
  assert.strictEqual(first.notice,notice);
  assert.equal(ordinary.length,32,'CONTROL does not use mailbox capacity');
  assert.deepEqual(supervisor.take_next(ordinary),
    {channel:'MESSAGE',message:0n});
  assert.equal(ordinary.length,31);
  assert.equal(supervisor.take_control(),null);
  assert.strictEqual(supervisor.get_terminal(1),notice,
    'durable terminal journal survives delivery');
});

test('multiple queued CONTROL notices precede ordinary messages in registration-independent order',()=>{
  const supervisor=new SupervisorControl();
  const a=supervisor.register_actor(1,0),b=supervisor.register_actor(2,0);
  const second=supervisor.record_terminal(b,'failed','E_ACTOR_WORKER',1);
  const first=supervisor.record_terminal(a,'cancelled','E_ACTOR_CANCELLED',0);
  const normal=[41n,42n];
  assert.strictEqual(supervisor.take_next(normal).notice,second);
  assert.strictEqual(supervisor.take_next(normal).notice,first);
  assert.equal(supervisor.take_next(normal).channel,'MESSAGE');
});

test('wait_control wakes for host terminal transition without requiring mailbox space',async()=>{
  const {supervisor,actor}=make(0,{hold_ready:true});
  const pending=supervisor.wait_control();
  for(let i=0;i<32;i++)assert.equal(actor.admit(BigInt(i)),ACTOR_ADMISSION.OK);
  assert.equal(actor.admit(32n),ACTOR_ADMISSION.FULL);
  actor.cancel();
  const event=await pending;
  assert.equal(event.kind,'cancelled');
  assert.equal(event.code,'E_ACTOR_CANCELLED');
  assert.equal(supervisor.drain_control().length,0,
    'wake consumes exactly one event, never duplicates');
  assert.strictEqual(supervisor.get_terminal(1),event);
});

test('panicking receive publishes one panic CONTROL notice before rejecting get_done',async()=>{
  const {supervisor,actor,worker}=make(9,{panic:true});
  actor.admit(7n);
  const draining=actor.drain_and_close();
  await assert.rejects(draining,/E_ACTOR_PANIC_1/);
  const event=supervisor.take_control();
  assert.equal(event.kind,'panic');
  assert.equal(event.processed,0);
  assert.equal(event.code,'E_ACTOR_PANIC_1');
  assert.equal(event.actor_id,9);
  await assert.rejects(actor.get_done(),/E_ACTOR_PANIC_1/);
  worker.emit({kind:'done',processed:1});
  assert.equal(supervisor.take_control(),null);
});

test('handler failure and cancellation cannot generate contradictory CONTROL events',async()=>{
  const {supervisor,actor,worker}=make(1,{hold_ready:true});
  actor.cancel();
  worker.emit({kind:'error',message:'E_ACTOR_PANIC_1'});
  worker.emit({kind:'done',processed:0});
  actor.fail('E_ACTOR_WORKER');
  const events=supervisor.drain_control();
  assert.equal(events.length,1);
  assert.equal(events[0].kind,'cancelled');
  assert.equal(events[0].state,'cancelled');
  assert.equal(actor.state,'cancelled');
  await assert.rejects(actor.get_done(),/E_ACTOR_CANCELLED/);
});

test('stalled receive and missing terminal ack publish timeout on CONTROL',async()=>{
  for(const [opts,payload,code] of [
    [{hold_reply:true},true,'E_ACTOR_TIMEOUT'],
    [{hold_done:true},false,'E_ACTOR_DONE_TIMEOUT'],
    [{hold_ready:true},false,'E_ACTOR_READY_TIMEOUT'],
  ]) {
    const {actor,supervisor}=make(0,{...opts,timeout_ms:30});
    if(payload)actor.admit(1n);
    await assert.rejects(actor.drain_and_close(),new RegExp(code));
    const items=supervisor.drain_control();
    assert.equal(items.length,1);
    assert.equal(items[0].kind,'timeout');
    assert.equal(items[0].code,code);
    assert.equal(items[0].route,'CONTROL');
  }
});

test('invalid child done-count never certifies termination',async()=>{
  const {actor,supervisor}=make(0,{bad_done:true});
  actor.admit(1n);
  await assert.rejects(actor.drain_and_close(),/E_ACTOR_DONE_PROTOCOL/);
  const items=supervisor.drain_control();
  assert.equal(items.length,1);
  assert.equal(items[0].kind,'failure');
  assert.equal(items[0].state,'failed');
  assert.equal(items[0].processed,1);
  assert.equal(supervisor.get_terminal(1)?.kind,'failure');
});

test('full four-actor CONTROL reservation cannot be saturated by 128 messages',async()=>{
  const supervisor=new SupervisorControl({capacity:4,parent_id:71});
  const sessions=[];
  for(let handle=1;handle<=4;handle++){
    const worker=new FakeWorker();
    const actor=new ActorSession(worker,0,new Uint8Array(),{
      supervisor,actor_handle:handle,timeout_ms:1000,
    });
    for(let i=0;i<32;i++)actor.admit(BigInt(i));
    sessions.push(actor);
  }
  assert.equal(sessions.length,4);
  await Promise.all(sessions.map(actor=>actor.drain_and_close()));
  const notices=supervisor.drain_control();
  assert.equal(notices.length,4);
  assert.deepEqual(notices.map(n=>n.actor_handle),[1,2,3,4]);
  assert.ok(notices.every(n=>n.kind==='terminated'&&n.processed===32));
  assert.ok(notices.every(n=>n.parent_id===71));
  for(const actor of sessions)assert.equal(actor.worker.terminated,1);
});

test('mixed four-actor panic/cancellation/completion outcomes remain independent',async()=>{
  const supervisor=new SupervisorControl({capacity:4,parent_id:99});
  const specs=[{}, {panic:true}, {hold_ready:true}, {}];
  const actors=specs.map((spec,i)=>{
    const worker=new FakeWorker(spec);
    const actor=new ActorSession(worker,i,new Uint8Array(),{
      supervisor,actor_handle:i+1,timeout_ms:1000,
    });
    actor.admit(BigInt(i+1));
    return actor;
  });
  actors[2].cancel();
  const all=await Promise.allSettled(actors.map(a=>a.drain_and_close()));
  assert.deepEqual(all.map(x=>x.status),[
    'fulfilled','rejected','rejected','fulfilled',
  ]);
  const events=supervisor.drain_control();
  assert.equal(events.length,4);
  assert.equal(new Set(events.map(x=>x.actor_handle)).size,4);
  assert.deepEqual(events.map(x=>x.kind).sort(),
    ['cancelled','panic','terminated','terminated']);
  for(const a of actors)assert.equal(a.worker.terminated,1);
});

test('seeded terminal-event races never duplicate or contradict CONTROL',async()=>{
  let seed=0x73aca105;
  const next=()=>{seed=(Math.imul(seed,1664525)+1013904223)>>>0;return seed};
  for(let turn=0;turn<128;turn++){
    const {supervisor,worker,actor}=make(turn,{hold_ready:true});
    const task=actor.drain_and_close();
    if(next()&1){worker.emit({kind:'ready'});actor.cancel()}
    else {actor.cancel();worker.emit({kind:'ready'})}
    worker.emit({kind:'error',message:'E_ACTOR_PANIC_1'});
    worker.emit({kind:'done',processed:0});
    await assert.rejects(task,/E_ACTOR_CANCELLED/);
    const events=supervisor.drain_control();
    assert.equal(events.length,1,'turn '+turn);
    assert.equal(events[0].kind,'cancelled','turn '+turn);
    assert.equal(supervisor.get_terminal(1),events[0]);
    assert.equal(actor.state,'cancelled','turn '+turn);
  }
});


test('source-level supervisor handles are bounded and never alias actor slots', () => {
  const supervisor=new SupervisorControl({capacity:4});
  for(const id of [-1,-2,NaN,1.5,Infinity,undefined]) {
    assert.throws(()=>supervisor.create_supervisor(id),/E_CONTROL_SUPERVISOR_LIMIT/);
  }
  assert.deepEqual([0,1,2,3].map(t=>supervisor.create_supervisor(t)),[1,2,3,4]);
  assert.throws(()=>supervisor.create_supervisor(4),/E_CONTROL_SUPERVISOR_LIMIT/);
  assert.equal(supervisor.slots.size,0);
  assert.equal(supervisor.supervisors.size,4);
});

test('typed watch admission requires both real parent and child handles', async()=>{
  const {supervisor,actor}=make(42,{hold_ready:true});
  assert.equal(supervisor.create_supervisor(0),1);
  for(const [parent,child] of [
    [-1,1],[0,1],[2,1],[99,1],[NaN,1],[1,0],[1,-1],
    [1,2],[1,99],[1,NaN],[1,Infinity],[1,'1'],['1',1],
  ]) {
    assert.equal(supervisor.watch_actor(parent,child),1,
      String(parent)+':'+String(child));
  }
  assert.equal(supervisor.watch_actor(1,1),0);
  assert.equal(supervisor.watch_actor(1,1),2,'even same parent cannot rewatch');
  assert.equal(supervisor.create_supervisor(1),2);
  assert.equal(supervisor.watch_actor(2,1),2,
    'second parent cannot steal an already watched actor');
  actor.cancel();
  const record=supervisor.take_control();
  assert.equal(record.parent_id,1);
  assert.equal(record.actor_id,42);
  assert.equal(record.kind,'cancelled');
  assert.equal(supervisor.watch_actor(2,1),3,
    'no adoption is possible after terminal transition');
});

test('watched completion is attributed to selected source supervisor',async()=>{
  const {supervisor,actor}=make(2);
  const parent=supervisor.create_supervisor(11);
  assert.equal(parent,1);
  assert.equal(supervisor.watch_actor(parent,1),0);
  assert.equal(actor.admit(41n),ACTOR_ADMISSION.OK);
  const outcome=await actor.drain_and_close();
  assert.equal(outcome.processed,1);
  const notice=supervisor.take_control();
  assert.equal(notice.kind,'terminated');
  assert.equal(notice.parent_id,parent);
  assert.equal(notice.actor_handle,1);
  assert.equal(notice.actor_id,2);
  assert.equal(notice.processed,1);
  assert.ok(Object.isFrozen(notice));
});

test('unwatched child remains assigned to the run-scoped parent',async()=>{
  const {supervisor,actor}=make(8);
  supervisor.create_supervisor(99);
  await actor.drain_and_close();
  const record=supervisor.take_control();
  assert.equal(record.parent_id,123,
    'merely creating a source supervisor never implicitly adopts children');
  assert.equal(record.actor_id,8);
});

test('a watched handler panic is routed to parent, never child code',async()=>{
  const {supervisor,actor}=make(7,{panic:true});
  const owner=supervisor.create_supervisor(0);
  assert.equal(supervisor.watch_actor(owner,1),0);
  actor.admit(41n);
  await assert.rejects(actor.drain_and_close(),/E_ACTOR_PANIC_1/);
  const event=supervisor.take_control();
  assert.equal(event.kind,'panic');
  assert.equal(event.parent_id,owner);
  assert.equal(event.code,'E_ACTOR_PANIC_1');
  assert.equal(event.processed,0);
  assert.equal(supervisor.take_control(),null);
});

test('watching a child whose ordinary mailbox is full does not consume a message slot',async()=>{
  const {supervisor,actor}=make(1,{hold_ready:true});
  for(let i=0;i<32;i++)assert.equal(actor.admit(BigInt(i)),ACTOR_ADMISSION.OK);
  assert.equal(actor.admit(32n),ACTOR_ADMISSION.FULL);
  const owner=supervisor.create_supervisor(1);
  assert.equal(supervisor.watch_actor(owner,1),0);
  assert.equal(actor.queue.length,32,'supervisor operation does not drain or admit data');
  actor.cancel();
  const record=supervisor.take_control();
  assert.equal(record.parent_id,owner);
  assert.equal(record.kind,'cancelled');
  assert.equal(actor.queue.length,0);
});

test('independent source supervisors cannot steal each others child notices',async()=>{
  const control=new SupervisorControl({capacity:4,parent_id:77});
  const first=control.create_supervisor(0),second=control.create_supervisor(1);
  const workers=[new FakeWorker(),new FakeWorker()];
  const actors=workers.map((worker,index)=>new ActorSession(worker,index,new Uint8Array(),{
    supervisor:control,actor_handle:index+1,timeout_ms:500,
  }));
  assert.equal(control.watch_actor(first,1),0);
  assert.equal(control.watch_actor(second,2),0);
  assert.equal(control.watch_actor(first,2),2);
  assert.equal(control.watch_actor(second,1),2);
  actors[0].admit(41n);
  actors[1].admit(42n);
  await Promise.all(actors.map(a=>a.drain_and_close()));
  const events=control.drain_control();
  assert.equal(events.length,2);
  assert.deepEqual(events.map(e=>e.parent_id),[first,second]);
  assert.deepEqual(events.map(e=>e.actor_handle),[1,2]);
  assert.ok(events.every(e=>e.kind==='terminated'));
});

test('terminal watch races preserve first committed parent attribution in 128 interleavings',async()=>{
  let seed=0x482ac0de;
  const rand=()=>{seed=(Math.imul(seed,1664525)+1013904223)>>>0;return seed};
  for(let turn=0;turn<128;turn++){
    const {supervisor,actor}=make(turn,{hold_ready:true});
    const parent=supervisor.create_supervisor(0);
    const before=(rand()&1)===0;
    if(before)assert.equal(supervisor.watch_actor(parent,1),0);
    actor.cancel();
    if(!before)assert.equal(supervisor.watch_actor(parent,1),3);
    const event=supervisor.take_control();
    assert.equal(event.parent_id,before?parent:123,'turn '+turn);
    assert.equal(event.kind,'cancelled');
    assert.equal(supervisor.take_control(),null);
    assert.strictEqual(supervisor.get_terminal(1),event);
  }
});


test('a forged seal token cannot close another actor watch admission',()=>{
  const {supervisor,actor}=make(0,{hold_ready:true});
  for(const token of [null,{},Object.freeze({}),[],1,'1',()=>{}]){
    assert.throws(()=>supervisor.seal_actor_watch(token),/E_CONTROL_CAPABILITY/);
  }
  const parent=supervisor.create_supervisor(0);
  assert.equal(supervisor.watch_actor(parent,1),0,
    'forged seal must not change admission state');
  actor.cancel();
});

test('a watched child cannot be reparented after drain is initiated',async()=>{
  const {supervisor,actor,worker}=make(5,{hold_ready:true});
  const first=supervisor.create_supervisor(0);
  const second=supervisor.create_supervisor(1);
  assert.equal(supervisor.watch_actor(first,1),0);
  const task=actor.drain_and_close();
  assert.equal(supervisor.watch_actor(second,1),4,
    'sealed in-flight actor must not be reparented before terminal acknowledgment');
  worker.emit({kind:'ready'});
  actor.cancel();
  await assert.rejects(task,/E_ACTOR_CANCELLED/);
  assert.equal(supervisor.take_control().parent_id,first);
});

test('unwatched draining child cannot be adopted by a late supervisor',async()=>{
  const {supervisor,actor,worker}=make(5,{hold_ready:true});
  const parent=supervisor.create_supervisor(0);
  const task=actor.drain_and_close();
  assert.equal(supervisor.watch_actor(parent,1),4);
  actor.cancel();
  worker.emit({kind:'ready'});
  await assert.rejects(task,/E_ACTOR_CANCELLED/);
  const notice=supervisor.take_control();
  assert.equal(notice.parent_id,123);
  assert.equal(notice.kind,'cancelled');
});

test('repeat sealing is idempotent and cannot undo an admitted parent',async()=>{
  const {supervisor,actor,worker}=make(7,{hold_ready:true});
  const parent=supervisor.create_supervisor(1);
  assert.equal(supervisor.watch_actor(parent,1),0);
  supervisor.seal_actor_watch(actor.control_token);
  supervisor.seal_actor_watch(actor.control_token);
  assert.equal(supervisor.watch_actor(parent,1),4);
  const task=actor.drain_and_close();
  worker.emit({kind:'ready'});
  actor.cancel();
  await assert.rejects(task,/E_ACTOR_CANCELLED/);
  assert.equal(supervisor.take_control().parent_id,parent);
});

test('watched in-flight worker failure retains the first parent attribution',async()=>{
  const {supervisor,actor,worker}=make(1,{hold_reply:true});
  const parent=supervisor.create_supervisor(3);
  assert.equal(supervisor.watch_actor(parent,1),0);
  actor.admit(41n);
  const task=actor.drain_and_close();
  await tick();
  worker.emit({kind:'error',message:'E_ACTOR_WORKER'});
  await assert.rejects(task,/E_ACTOR_WORKER/);
  const event=supervisor.take_control();
  assert.equal(event.parent_id,parent);
  assert.equal(event.kind,'failure');
  assert.equal(event.processed,0);
  assert.equal(supervisor.watch_actor(parent,1),3);
});

test('watched readiness timeout preserves selected parent in CONTROL',async()=>{
  const {supervisor,actor}=make(0,{hold_ready:true,timeout_ms:25});
  const parent=supervisor.create_supervisor(0);
  assert.equal(supervisor.watch_actor(parent,1),0);
  await assert.rejects(actor.drain_and_close(),/E_ACTOR_READY_TIMEOUT/);
  const event=supervisor.take_control();
  assert.equal(event.parent_id,parent);
  assert.equal(event.kind,'timeout');
  assert.equal(event.code,'E_ACTOR_READY_TIMEOUT');
});

test('watched close timeout never turns into successful parent event',async()=>{
  const {supervisor,actor}=make(0,{hold_done:true,timeout_ms:25});
  const parent=supervisor.create_supervisor(0);
  assert.equal(supervisor.watch_actor(parent,1),0);
  await assert.rejects(actor.drain_and_close(),/E_ACTOR_DONE_TIMEOUT/);
  const event=supervisor.take_control();
  assert.equal(event.parent_id,parent);
  assert.equal(event.kind,'timeout');
  assert.equal(event.state,'failed');
});

test('watched malformed reply fails closed without reassigning parent',async()=>{
  const {supervisor,actor,worker}=make(0,{hold_reply:true});
  const parent=supervisor.create_supervisor(0);
  assert.equal(supervisor.watch_actor(parent,1),0);
  actor.admit(11n);
  const task=actor.drain_and_close();
  await tick();
  worker.emit({kind:'reply',seq:99,value:12n});
  await assert.rejects(task,/E_ACTOR_REPLY_PROTOCOL/);
  const event=supervisor.take_control();
  assert.equal(event.parent_id,parent);
  assert.equal(event.kind,'failure');
  assert.equal(event.processed,0);
  assert.equal(supervisor.take_control(),null);
});

test('watched worker-level clone error preserves CONTROL parent and exactly-once delivery',async()=>{
  const {supervisor,actor,worker}=make(5,{hold_ready:true});
  const parent=supervisor.create_supervisor(2);
  assert.equal(supervisor.watch_actor(parent,1),0);
  worker.onmessageerror?.({});
  worker.emit({kind:'ready'});
  actor.cancel();
  await assert.rejects(actor.get_done(),/E_ACTOR_MESSAGE_CLONE/);
  const event=supervisor.take_control();
  assert.equal(event.parent_id,parent);
  assert.equal(event.code,'E_ACTOR_MESSAGE_CLONE');
  assert.equal(supervisor.take_control(),null);
  assert.strictEqual(supervisor.get_terminal(1),event);
});

test('four independently watched actors preserve one source supervisor each',async()=>{
  const control=new SupervisorControl({capacity:4,parent_id:0});
  const parents=Array.from({length:4},(_,i)=>control.create_supervisor(i));
  const actors=Array.from({length:4},(_,i)=>{
    const worker=new FakeWorker();
    const actor=new ActorSession(worker,i,new Uint8Array(),{
      supervisor:control,actor_handle:i+1,timeout_ms:1000,
    });
    assert.equal(control.watch_actor(parents[i],i+1),0);
    actor.admit(BigInt(i));
    return actor;
  });
  await Promise.all(actors.map(a=>a.drain_and_close()));
  const records=control.drain_control();
  assert.equal(records.length,4);
  assert.deepEqual(records.map(r=>r.parent_id),parents);
  assert.deepEqual(records.map(r=>r.actor_handle),[1,2,3,4]);
  assert.ok(records.every(r=>r.kind==='terminated'&&r.processed===1));
});

test('fifth supervisor creation cannot corrupt four prior parent assignments',async()=>{
  const control=new SupervisorControl({capacity:4,parent_id:0});
  const parents=Array.from({length:4},(_,i)=>control.create_supervisor(i));
  const worker=new FakeWorker();
  const actor=new ActorSession(worker,0,new Uint8Array(),{
    supervisor:control,actor_handle:1,
  });
  assert.equal(control.watch_actor(parents[3],1),0);
  assert.throws(()=>control.create_supervisor(100),/E_CONTROL_SUPERVISOR_LIMIT/);
  assert.equal(control.supervisors.size,4);
  await actor.drain_and_close();
  assert.equal(control.take_control().parent_id,parents[3]);
});

test('128 seeded watch versus structured-close interleavings never reparent terminal events',async()=>{
  let seed=0x83fe47d1;
  const rand=()=>{seed=(Math.imul(seed,1664525)+1013904223)>>>0;return seed};
  for(let i=0;i<128;i++){
    const {actor,supervisor}=make(i,{hold_ready:true});
    const parent=supervisor.create_supervisor(0);
    const before=(rand()&1)===0;
    if(before)assert.equal(supervisor.watch_actor(parent,1),0);
    const task=actor.drain_and_close();
    if(!before)assert.equal(supervisor.watch_actor(parent,1),4);
    actor.cancel();
    await assert.rejects(task,/E_ACTOR_CANCELLED/);
    const record=supervisor.take_control();
    assert.equal(record.parent_id,before?parent:123,'iteration '+i);
    assert.equal(record.state,'cancelled');
    assert.equal(supervisor.take_control(),null);
  }
});

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

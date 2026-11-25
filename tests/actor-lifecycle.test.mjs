// Node tests for the same lifecycle state machine used by real Chromium.
// Fake workers are injected *only* here for precise protocol/race controls.
import test from 'node:test';
import assert from 'node:assert/strict';
import { ActorSession, ACTOR_ADMISSION } from './browser/actor-lifecycle.mjs';

class FakeActorWorker {
  constructor(options = {}) {
    this.options = options;
    this.messages = [];
    this.terminated = 0;
    this.received = 0;
  }
  emit(data) {
    this.onmessage?.({data});
  }
  postMessage(msg) {
    this.messages.push(msg);
    if (msg.kind === 'init') {
      if (!this.options.hold_ready) queueMicrotask(() => this.emit({kind:'ready'}));
      return;
    }
    if (msg.kind === 'message') {
      if (this.options.panic) {
        queueMicrotask(() => this.emit({kind:'error',message:'E_ACTOR_PANIC_1'}));
      } else if (!this.options.hold_reply) {
        this.received++;
        queueMicrotask(() => this.emit({kind:'reply',seq:msg.seq,
          value:this.options.identity?msg.value:msg.value+1n}));
      }
      return;
    }
    if (msg.kind === 'close' && !this.options.hold_done) {
      const processed = this.received + (this.options.bad_done_count ? 1 : 0);
      queueMicrotask(() => this.emit({kind:'done',processed}));
    }
  }
  terminate() {
    this.terminated++;
  }
}

test('get_done is pending until an explicit terminal worker acknowledgement', async () => {
  const worker = new FakeActorWorker({hold_done:true});
  const actor = new ActorSession(worker,0,new Uint8Array(),{timeout_ms:300});
  assert.strictEqual(actor.get_done(),actor.get_done(),'one stable completion task');
  assert.equal(actor.admit(41n),ACTOR_ADMISSION.OK);
  const observed=[];
  const draining=actor.drain_and_close((message,reply)=>observed.push([message,reply]));
  await new Promise(resolve=>setImmediate(resolve));
  assert.equal(actor.state,'closing');
  assert.equal(actor.admit(42n),ACTOR_ADMISSION.CLOSED);
  let settled=false;
  void actor.get_done().then(()=>{settled=true;},()=>{settled=true;});
  await Promise.resolve();
  assert.equal(settled,false,'mailbox admission and reply do not complete lifecycle');
  assert.deepEqual(worker.messages.map(x=>x.kind),['init','message','close']);
  worker.emit({kind:'done',processed:1});
  assert.deepEqual(await draining,{kind:'completed',actor_id:0,processed:1});
  assert.deepEqual(observed,[[41n,42n]]);
  assert.equal(actor.state,'completed');
  assert.equal(settled,true);
  const count=worker.terminated;
  worker.emit({kind:'error',message:'E_ACTOR_PANIC_1'});
  assert.equal(actor.state,'completed','late errors cannot mutate terminal result');
  assert.equal(worker.terminated,count);
});

test('bounded admission is linearized before async delivery', async () => {
  const worker=new FakeActorWorker();
  const actor=new ActorSession(worker,1,new Uint8Array(),{timeout_ms:500});
  for(let i=0;i<32;i++)assert.equal(actor.admit(BigInt(i)),ACTOR_ADMISSION.OK);
  assert.equal(actor.admit(32n),ACTOR_ADMISSION.FULL);
  assert.equal(actor.admit({pointer:42}),ACTOR_ADMISSION.INVALID);
  const replies=[];
  const result=await actor.drain_and_close((value,reply)=>replies.push([value,reply]));
  assert.equal(result.processed,32);
  assert.equal(worker.messages.filter(x=>x.kind==='message').length,32);
  assert.deepEqual(replies.map(x=>x[0]),Array.from({length:32},(_,i)=>BigInt(i)));
  assert.equal(actor.admit(33n),ACTOR_ADMISSION.CLOSED);
});

test('actor panic rejects task, stops worker, never resolves success', async () => {
  const worker=new FakeActorWorker({panic:true});
  const actor=new ActorSession(worker,0,new Uint8Array());
  actor.admit(1n);
  await assert.rejects(actor.drain_and_close(),/E_ACTOR_PANIC_1/);
  await assert.rejects(actor.get_done(),/E_ACTOR_PANIC_1/);
  assert.equal(actor.state,'failed');
  assert.equal(worker.terminated,1);
  worker.emit({kind:'done',processed:1});
  await assert.rejects(actor.get_done(),/E_ACTOR_PANIC_1/);
});

test('cancellation before readiness rejects all future completion and admissions', async () => {
  const worker=new FakeActorWorker({hold_ready:true});
  const actor=new ActorSession(worker,2,new Uint8Array());
  actor.admit(1n);
  const task=actor.get_done();
  actor.cancel();
  worker.emit({kind:'ready'});
  worker.emit({kind:'done',processed:0});
  await assert.rejects(task,/E_ACTOR_CANCELLED/);
  await assert.rejects(actor.drain_and_close(),/E_ACTOR_CANCELLED/);
  assert.equal(actor.state,'cancelled');
  assert.equal(actor.admit(2n),ACTOR_ADMISSION.CLOSED);
});

test('missing terminal acknowledgement cannot be mistaken for completion', async () => {
  const worker=new FakeActorWorker({hold_done:true});
  const actor=new ActorSession(worker,0,new Uint8Array(),{timeout_ms:35});
  await assert.rejects(actor.drain_and_close(),/E_ACTOR_DONE_TIMEOUT/);
  await assert.rejects(actor.get_done(),/E_ACTOR_DONE_TIMEOUT/);
  assert.equal(actor.state,'failed');
  assert.equal(worker.terminated,1);
});

test('incorrect final count fails closed even after all replies', async () => {
  const worker=new FakeActorWorker({bad_done_count:true});
  const actor=new ActorSession(worker,0,new Uint8Array());
  actor.admit(12n);
  await assert.rejects(actor.drain_and_close(),/E_ACTOR_DONE_PROTOCOL/);
  await assert.rejects(actor.get_done(),/E_ACTOR_DONE_PROTOCOL/);
  assert.equal(actor.state,'failed');
});

test('unsolicited reply and unexpected protocol are terminal faults', async () => {
  const worker=new FakeActorWorker({hold_ready:true});
  const actor=new ActorSession(worker,0,new Uint8Array());
  worker.emit({kind:'reply',seq:0,value:10n});
  await assert.rejects(actor.get_done(),/E_ACTOR_REPLY_PROTOCOL/);
  assert.equal(actor.state,'failed');
});

test('a stuck receive times out without blocking the parent event loop', async () => {
  const worker=new FakeActorWorker({hold_reply:true});
  const actor=new ActorSession(worker,0,new Uint8Array(),{timeout_ms:35});
  actor.admit(7n);
  let ticks=0;
  queueMicrotask(()=>ticks++);
  await assert.rejects(actor.drain_and_close(),/E_ACTOR_TIMEOUT/);
  assert.ok(ticks>0,'the parent event loop remained runnable');
  assert.equal(actor.state,'failed');
  assert.equal(worker.terminated,1);
});


const tick = () => new Promise(resolve => setImmediate(resolve));

test('session creation validates worker identity, IDs, and timeout quotas', () => {
  for (const [worker, id, timeout] of [
    [null, 0, 100], [false, 0, 100], [{}, -1, 100],
    [{}, NaN, 100], [{}, 1.5, 100], [{}, 0, 0],
    [{}, 0, -1], [{}, 0, Infinity], [{}, 0, 1.1],
  ]) {
    assert.throws(
      () => new ActorSession(worker, id, new Uint8Array(), {timeout_ms:timeout}),
      /E_ACTOR_SESSION_ARGS/,
    );
  }
});

test('zero-message actor cannot resolve before its explicit close ack', async () => {
  const worker = new FakeActorWorker({hold_done:true});
  const actor = new ActorSession(worker,0,new Uint8Array());
  let settled = false;
  void actor.get_done().then(() => {settled=true;}, () => {settled=true;});
  const task = actor.drain_and_close();
  await tick();
  assert.equal(actor.state,'closing');
  assert.equal(settled,false);
  assert.deepEqual(worker.messages.map(x => x.kind),['init','close']);
  worker.emit({kind:'done',processed:0});
  assert.deepEqual(await task,{kind:'completed',actor_id:0,processed:0});
  assert.equal(worker.terminated,1);
});

test('a duplicate drain call is the same task and never sends a second close', async () => {
  const worker=new FakeActorWorker();
  const actor=new ActorSession(worker,0,new Uint8Array());
  actor.admit(1n);
  const p=actor.drain_and_close();
  assert.strictEqual(actor.drain_and_close(),p);
  assert.equal((await p).processed,1);
  assert.strictEqual(actor.drain_and_close(),p);
  assert.equal(worker.messages.filter(x=>x.kind==='close').length,1);
  assert.equal(worker.terminated,1);
});

test('admission treats signed i64 boundaries as values, never pointer handles', async () => {
  const actor=new ActorSession(new FakeActorWorker({identity:true}),0,new Uint8Array());
  const min=-(1n<<63n), max=(1n<<63n)-1n;
  for(const value of [min, max, 0n])assert.equal(actor.admit(value),ACTOR_ADMISSION.OK);
  for(const value of [
    min-1n, max+1n, 2n**128n, -(2n**128n), null, undefined,
    0, 1.5, true, '41', [], {}, {ptr:1}, new Uint8Array([42]),
  ]) {
    assert.equal(actor.admit(value),ACTOR_ADMISSION.INVALID,String(value));
  }
  // An identity-echo worker avoids overflowing the max i64 on reply.
  // The transport preserves both i64 extrema; pointers are not messages.
  const messages=[];
  const outcome=await actor.drain_and_close((message)=>messages.push(message));
  assert.equal(outcome.processed,3);
  assert.deepEqual(messages,[min,max,0n]);
});

test('malformed or replayed sequence numbers reject a pending receive', async () => {
  for(const seq of [null, undefined, -1, 1, 0.5, '0', NaN, 2**53]) {
    const worker=new FakeActorWorker({hold_reply:true});
    const actor=new ActorSession(worker,0,new Uint8Array());
    actor.admit(41n);
    const task=actor.drain_and_close();
    await tick();
    assert.equal(actor.pending?.seq,0);
    worker.emit({kind:'reply',seq,value:42n});
    await assert.rejects(task,/E_ACTOR_REPLY_PROTOCOL/);
    await assert.rejects(actor.get_done(),/E_ACTOR_REPLY_PROTOCOL/);
    assert.equal(actor.state,'failed');
    assert.equal(worker.terminated,1);
  }
});

test('a stale reply for message 0 cannot satisfy the next pending message', async () => {
  const worker=new FakeActorWorker({hold_reply:true});
  const actor=new ActorSession(worker,0,new Uint8Array());
  actor.admit(1n);
  actor.admit(2n);
  const observed=[];
  const task=actor.drain_and_close((value,reply)=>observed.push([value,reply]));
  await tick();
  assert.equal(actor.pending?.seq,0);
  worker.emit({kind:'reply',seq:0,value:2n});
  await tick();
  assert.equal(actor.pending?.seq,1);
  assert.deepEqual(observed,[[1n,2n]]);
  worker.emit({kind:'reply',seq:0,value:2n}); // replay of old answer
  await assert.rejects(task,/E_ACTOR_REPLY_PROTOCOL/);
  assert.equal(actor.processed,1,'replay does not count as the second reply');
  assert.equal(actor.state,'failed');
});

test('a future reply cannot acknowledge an earlier message', async () => {
  const worker=new FakeActorWorker({hold_reply:true});
  const actor=new ActorSession(worker,0,new Uint8Array());
  actor.admit(10n);
  const task=actor.drain_and_close();
  await tick();
  worker.emit({kind:'reply',seq:1,value:11n});
  await assert.rejects(task,/E_ACTOR_REPLY_PROTOCOL/);
  assert.equal(actor.processed,0);
  worker.emit({kind:'reply',seq:0,value:11n});
  await assert.rejects(actor.get_done(),/E_ACTOR_REPLY_PROTOCOL/);
});

test('reply value must be a canonical signed i64 regardless of matching sequence', async () => {
  const over=(1n<<63n);
  for(const invalid of [over,-over-1n,'42',42,{},null,undefined,1.5]) {
    const worker=new FakeActorWorker({hold_reply:true});
    const actor=new ActorSession(worker,0,new Uint8Array());
    actor.admit(1n);
    const task=actor.drain_and_close();
    await tick();
    worker.emit({kind:'reply',seq:0,value:invalid});
    await assert.rejects(task,/E_ACTOR_REPLY_PROTOCOL/);
    assert.equal(actor.processed,0);
  }
});

test('unexpected done during a pending message is a protocol error', async () => {
  const worker=new FakeActorWorker({hold_reply:true});
  const actor=new ActorSession(worker,0,new Uint8Array());
  actor.admit(11n);
  const task=actor.drain_and_close();
  await tick();
  worker.emit({kind:'done',processed:1});
  await assert.rejects(task,/E_ACTOR_DONE_PROTOCOL/);
  assert.equal(actor.state,'failed');
});

test('ready cannot repeat and unknown messages fail closed', async () => {
  for(const injected of [{kind:'ready'},{kind:'undocumented'},null]) {
    const worker=new FakeActorWorker();
    const actor=new ActorSession(worker,0,new Uint8Array());
    await actor.ready;
    worker.emit(injected);
    await assert.rejects(actor.get_done(),/E_ACTOR_PROTOCOL/);
    assert.equal(actor.state,'failed');
    assert.equal(worker.terminated,1);
  }
});

test('invalid child error messages are not trusted as host error codes', async () => {
  for(const message of ['panic 1','E_bad','E_X\nINJECT','E_'+('A'.repeat(100000)),undefined]) {
    const worker=new FakeActorWorker({hold_ready:true});
    const actor=new ActorSession(worker,0,new Uint8Array());
    worker.emit({kind:'error',message});
    await assert.rejects(actor.get_done(),/E_ACTOR_FAULT/);
    assert.equal(actor.state,'failed');
  }
});

test('unresponsive initialization times out without resolving done', async () => {
  const worker=new FakeActorWorker({hold_ready:true});
  const actor=new ActorSession(worker,0,new Uint8Array(),{timeout_ms:30});
  await assert.rejects(actor.get_done(),/E_ACTOR_READY_TIMEOUT/);
  assert.equal(actor.state,'failed');
  assert.equal(worker.terminated,1);
});

test('worker-level failures terminate exactly once and reject task', async () => {
  for(const [event,code] of [
    ['onerror','E_ACTOR_WORKER'],['onmessageerror','E_ACTOR_MESSAGE_CLONE'],
  ]) {
    const worker=new FakeActorWorker({hold_ready:true});
    const actor=new ActorSession(worker,0,new Uint8Array());
    worker[event]?.({message:'malformed'});
    await assert.rejects(actor.get_done(),new RegExp(code));
    actor.cancel();
    worker.emit({kind:'done',processed:0});
    assert.equal(worker.terminated,1);
    assert.equal(actor.state,'failed');
  }
});

test('message cloning and close cloning errors reject instead of falsely completing', async () => {
  for(const kind of ['message','close']) {
    const worker=new FakeActorWorker();
    const send=worker.postMessage.bind(worker);
    worker.postMessage=(msg)=>{
      if(msg.kind===kind)throw new Error('synthetic clone fault');
      return send(msg);
    };
    const actor=new ActorSession(worker,0,new Uint8Array());
    if(kind==='message')actor.admit(1n);
    await assert.rejects(actor.drain_and_close(),
      new RegExp(kind==='message'?'E_ACTOR_MESSAGE_CLONE':'synthetic clone fault'));
    await assert.rejects(actor.get_done(),new RegExp(
      kind==='message'?'E_ACTOR_MESSAGE_CLONE':'E_ACTOR_DRAIN_FAULT'));
    assert.equal(actor.state,'failed');
    assert.equal(worker.terminated,1);
  }
});

test('constructor postMessage fault cannot leak a running child', async () => {
  const worker=new FakeActorWorker();
  worker.postMessage=()=>{throw new Error('synthetic init clone fault')};
  const actor=new ActorSession(worker,0,new Uint8Array());
  await assert.rejects(actor.get_done(),/E_ACTOR_INIT_CLONE/);
  assert.equal(actor.state,'failed');
  assert.equal(worker.terminated,1);
});

test('cancel during in-flight message rejects await and ignores stale replies', async () => {
  const worker=new FakeActorWorker({hold_reply:true});
  const actor=new ActorSession(worker,0,new Uint8Array());
  actor.admit(41n);
  const task=actor.drain_and_close();
  await tick();
  assert.equal(actor.state,'draining');
  actor.cancel();
  worker.emit({kind:'reply',seq:0,value:42n});
  worker.emit({kind:'done',processed:1});
  await assert.rejects(task,/E_ACTOR_CANCELLED/);
  await assert.rejects(actor.get_done(),/E_ACTOR_CANCELLED/);
  assert.equal(actor.state,'cancelled');
  assert.equal(actor.processed,0);
  assert.equal(actor.admit(43n),ACTOR_ADMISSION.CLOSED);
  assert.equal(worker.terminated,1);
});

test('on_reply callback exceptions fail the task without fabricating success', async () => {
  const actor=new ActorSession(new FakeActorWorker(),0,new Uint8Array());
  actor.admit(1n);
  await assert.rejects(actor.drain_and_close(()=>{throw new Error('observer failed')}),/observer failed/);
  await assert.rejects(actor.get_done(),/E_ACTOR_DRAIN_FAULT/);
  assert.equal(actor.state,'failed');
});

test('four independently scoped workers cannot share completion or failure', async () => {
  const actors=Array.from({length:4},(_,id)=>{
    const worker=new FakeActorWorker(id===1?{panic:true}:{});
    const actor=new ActorSession(worker,id,new Uint8Array());
    actor.admit(BigInt(id));
    return actor;
  });
  const outcomes=await Promise.allSettled(actors.map(a=>a.drain_and_close()));
  assert.deepEqual(outcomes.map(x=>x.status),['fulfilled','rejected','fulfilled','fulfilled']);
  assert.equal(actors[0].state,'completed');
  assert.equal(actors[1].state,'failed');
  assert.equal(actors[2].state,'completed');
  assert.equal(actors[3].state,'completed');
  for(const a of actors)assert.equal(a.worker.terminated,1);
});

test('deterministic 32-turn FIFO model matches real state-machine admission', async () => {
  let seed=0x4fdc2015;
  const next=()=>{seed=(Math.imul(seed,1664525)+1013904223)>>>0;return seed};
  for(let iteration=0;iteration<32;iteration++){
    const actor=new ActorSession(new FakeActorWorker(),iteration,new Uint8Array());
    const input=[];
    const length=next()%40;  // deliberately includes overcapacity
    for(let i=0;i<length;i++){
      const value=BigInt((next()%200000)-100000);
      const code=actor.admit(value);
      assert.equal(code,i<32?ACTOR_ADMISSION.OK:ACTOR_ADMISSION.FULL);
      if(i<32)input.push(value);
    }
    const observed=[];
    const outcome=await actor.drain_and_close((value,reply)=>observed.push([value,reply]));
    assert.equal(outcome.processed,input.length);
    assert.deepEqual(observed,input.map(x=>[x,x+1n]));
    assert.equal(actor.queue.length,0);
    assert.equal(actor.admit(1n),ACTOR_ADMISSION.CLOSED);
  }
});


test('cancellation in ready-resolution microtask cannot restart a terminal actor', async () => {
  const worker=new FakeActorWorker({hold_ready:true});
  const actor=new ActorSession(worker,0,new Uint8Array(),{timeout_ms:150});
  assert.equal(actor.admit(41n),ACTOR_ADMISSION.OK);
  const task=actor.drain_and_close();
  // resolves ready, scheduling the drain continuation, but cancel wins before
  // the continuation executes; pre-fix it overwrote cancelled with closing.
  worker.emit({kind:'ready'});
  actor.cancel();
  await assert.rejects(task,/E_ACTOR_CANCELLED/);
  await assert.rejects(actor.get_done(),/E_ACTOR_CANCELLED/);
  assert.equal(actor.state,'cancelled');
  assert.equal(actor.processed,0);
  assert.equal(actor.worker.terminated,1);
  assert.deepEqual(worker.messages.map(x=>x.kind),['init'],
    'no message or close may be sent to a cancelled child');
  worker.emit({kind:'done',processed:0});
  assert.equal(actor.state,'cancelled');
});

test('cancellation immediately after a resolved reply prevents callback and close', async () => {
  const worker=new FakeActorWorker({hold_reply:true});
  const actor=new ActorSession(worker,0,new Uint8Array(),{timeout_ms:150});
  assert.equal(actor.admit(7n),ACTOR_ADMISSION.OK);
  const observed=[];
  const task=actor.drain_and_close((msg,reply)=>observed.push([msg,reply]));
  await tick();
  assert.equal(actor.pending?.seq,0);
  worker.emit({kind:'reply',seq:0,value:8n});
  actor.cancel(); // immediately before await send_to_worker continuation
  await assert.rejects(task,/E_ACTOR_CANCELLED/);
  assert.equal(actor.state,'cancelled');
  assert.deepEqual(observed,[],'no observation after terminal cancellation');
  assert.equal(worker.messages.filter(x=>x.kind==='close').length,0);
  assert.equal(actor.processed,1,'reply happened before cancellation');
  assert.equal(actor.worker.terminated,1);
});

test('observer-initiated cancellation cannot be overwritten by closing', async () => {
  const worker=new FakeActorWorker();
  const actor=new ActorSession(worker,0,new Uint8Array());
  actor.admit(11n);
  const task=actor.drain_and_close((msg,reply)=>{
    assert.deepEqual([msg,reply],[11n,12n]);
    actor.cancel();
  });
  await assert.rejects(task,/E_ACTOR_CANCELLED/);
  await assert.rejects(actor.get_done(),/E_ACTOR_CANCELLED/);
  assert.equal(actor.state,'cancelled');
  assert.equal(worker.messages.filter(x=>x.kind==='close').length,0);
  assert.equal(worker.terminated,1);
});

test('cancel between two admitted sends never dispatches a later message', async () => {
  const worker=new FakeActorWorker();
  const actor=new ActorSession(worker,0,new Uint8Array());
  actor.admit(1n);
  actor.admit(2n);
  const replies=[];
  const task=actor.drain_and_close((msg,reply)=>{
    replies.push([msg,reply]);
    actor.cancel();
  });
  await assert.rejects(task,/E_ACTOR_CANCELLED/);
  assert.deepEqual(replies,[[1n,2n]]);
  assert.deepEqual(
    worker.messages.filter(x=>x.kind==='message').map(x=>x.value),
    [1n],
    'the second admission was dropped on cancellation, never dispatched');
  assert.equal(actor.state,'cancelled');
  assert.equal(actor.admit(3n),ACTOR_ADMISSION.CLOSED);
});

test('first terminal cause is stable across repeated cancellation and stale messages', async () => {
  const worker=new FakeActorWorker({hold_reply:true});
  const actor=new ActorSession(worker,0,new Uint8Array());
  actor.admit(3n);
  const draining=actor.drain_and_close();
  await tick();
  actor.fail('E_ACTOR_WORKER');
  const first=await actor.get_done().then(()=>null,e=>e);
  assert.match(first.message,/E_ACTOR_WORKER/);
  actor.cancel();
  actor.fail('E_ACTOR_PANIC_1');
  worker.emit({kind:'reply',seq:0,value:4n});
  worker.emit({kind:'done',processed:1});
  await assert.rejects(draining,e=>e===first,
    'drain must fail with the original terminal error object');
  await assert.rejects(actor.get_done(),e=>e===first,
    'completion remains the first failure, never overwritten');
  assert.equal(actor.state,'failed');
  assert.equal(worker.terminated,1);
});

test('cancellation after genuine terminal ack is idempotent and cannot rewrite success', async () => {
  const worker=new FakeActorWorker();
  const actor=new ActorSession(worker,0,new Uint8Array());
  actor.admit(5n);
  const completed=await actor.drain_and_close();
  assert.deepEqual(completed,{kind:'completed',actor_id:0,processed:1});
  actor.cancel();
  actor.fail('E_ACTOR_WORKER');
  worker.emit({kind:'error',message:'E_ACTOR_PANIC_1'});
  assert.strictEqual(await actor.get_done(),completed);
  assert.equal(actor.state,'completed');
  assert.equal(worker.terminated,1);
});

test('cancelled actor never regains drain privileges under randomized ready races', async () => {
  let seed=0x9217b64d;
  const rand=()=>{seed=(Math.imul(seed,1664525)+1013904223)>>>0;return seed};
  for(let i=0;i<128;i++){
    const worker=new FakeActorWorker({hold_ready:true});
    const actor=new ActorSession(worker,i,new Uint8Array(),{timeout_ms:500});
    actor.admit(BigInt(i));
    const task=actor.drain_and_close();
    const order=rand()&1;
    if(order===0){
      actor.cancel();
      worker.emit({kind:'ready'});
    }else{
      worker.emit({kind:'ready'});
      actor.cancel();
    }
    await assert.rejects(task,/E_ACTOR_CANCELLED/);
    assert.equal(actor.state,'cancelled','iteration '+i);
    assert.deepEqual(worker.messages.map(x=>x.kind),['init'],'iteration '+i);
    assert.equal(worker.terminated,1,'iteration '+i);
  }
});

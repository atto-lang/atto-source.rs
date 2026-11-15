// Real native-Rust/browser-Wasm compiler -> emitted guest -> Node Workers.
// A dedicated Worker per actor is intentional; this is not pooled scheduler proof.
import assert from 'node:assert/strict';
import {Worker} from 'node:worker_threads';
import {readFileSync} from 'node:fs';
import {M9SupervisorHost, notice_to_wire_json} from '../runtime/m9-supervisor-host.mjs';
const compilerBytes=readFileSync('target/wasm32-unknown-unknown/release/atto_compiler.wasm');
const compiler=new WebAssembly.Instance(new WebAssembly.Module(compilerBytes)).exports;
assert.equal(compiler.abi_version(),2);
const source=readFileSync('tests/fixtures/m8/cooperative_sum.atto','utf8');
function compile(src) {
  const data=new TextEncoder().encode(src);
  new Uint8Array(compiler.memory.buffer,compiler.input_ptr(),data.length).set(data);
  const status=compiler.compile(data.length);
  assert.equal(status,0,'Rust/Wasm compiler rejected expected M9 actor');
  const out=new Uint8Array(compiler.memory.buffer,compiler.result_ptr(),compiler.result_length()).slice();
  assert.ok(WebAssembly.validate(out));
  return out;
}
const healthy=compile(source);
const faultSource=source.replace('let total: int = 0;','let total: int = 9223372036854775807;')
  .replace('let i: int = 0;','let i: int = 1;');
const fault=compile(faultSource);
function make_worker(){return new Worker(new URL('../runtime/m9-actor-worker.mjs',import.meta.url),{type:'module'})}
const stopGolden=readFileSync('tests/fixtures/m9/stop.json','utf8').trimEnd();
const panicGolden=readFileSync('tests/fixtures/m9/panic.json','utf8').trimEnd();
const stopped={kind:'STOP',actor_id:0,generation:1,supervisor_id:777,
  message_id:0,fault_code:0,reason:'graceful stop'};
const panicked={kind:'PANIC',actor_id:0,generation:1,supervisor_id:777,
  message_id:1,fault_code:1,reason:'checked arithmetic'};
assert.equal(notice_to_wire_json(stopped),stopGolden);
assert.equal(notice_to_wire_json(panicked),panicGolden);
for (const mutation of [
  {...stopped,actor_id:-1}, {...stopped,actor_id:Number.MAX_SAFE_INTEGER+1},
  {...stopped,generation:0}, {...panicked,fault_code:0},
  {...stopped,message_id:1}, {...panicked,reason:'x'.repeat(129)},
  {...panicked,kind:'DONE'}
])assert.throws(()=>notice_to_wire_json(mutation),'malformed wire accepted');
const notices=[];
const host=new M9SupervisorHost({maxActors:4,mailboxCapacity:3,reductionBudget:4,
  onControl:notice=>notices.push(notice)});
try {
  const workerA=host.spawn(make_worker(),healthy,21);
  const workerB=host.spawn(make_worker(),fault,21);
  assert.equal(host.try_send({...workerA},1),'STALE','forged ActorRef must not gain authority');
  assert.equal(host.try_send(workerA,10000),'ACCEPTED');
  assert.equal(host.try_send(workerA,12),'ACCEPTED');
  assert.equal(host.try_send(workerA,3),'ACCEPTED');
  assert.equal(host.try_send(workerB,3),'ACCEPTED');
  assert.equal(host.try_send(workerB,3),'ACCEPTED');
  assert.equal(host.try_send(workerB,3),'ACCEPTED');
  assert.equal(host.try_send(workerB,3),'FULL','bounded mailbox must saturate');
  assert.equal(host.request_stop(workerB),true,'STOP must be independently admitted on full DATA');
  assert.equal(host.request_stop(workerB),true,'repeated STOP coalesces');
  assert.equal(host.try_send(workerB,3),'CLOSED','STOP must fence new admission');
  assert.equal(host.request_stop(workerA),true);
  assert.equal(host.try_send(workerA,1),'CLOSED');
  assert.ok(await host.await_terminal(workerA,30000),'healthy actor was starved');
  assert.ok(await host.await_terminal(workerB,30000),'faulted actor did not terminate');
  const a=host.check_conservation(workerA),b=host.check_conservation(workerB);
  assert.equal(a.accepted,3);
  assert.equal(a.completed,3);
  assert.equal(a.failed,0);assert.equal(a.abandoned,0);
  assert.equal(a.resultSum,49995000n+66n+3n);
  assert.ok(a.yielded>=2499,'real compiled continuation did not yield');
  assert.equal(a.controlsDelivered,1);
  assert.equal(b.accepted,3);
  assert.equal(b.completed,0);
  assert.equal(b.failed,1);
  assert.equal(b.abandoned,2);
  assert.equal(b.lastControl,'PANIC');
  assert.equal(b.controlsDelivered,2,'STOP must upgrade to PANIC, not vanish');
  assert.equal(host.try_send(workerB,1),'STALE');
  const faultNotices=notices.filter(x=>x.actor_id===workerB.actor_id);
  assert.deepEqual(faultNotices.map(x=>x.kind),['STOP','PANIC']);
  assert.equal(faultNotices[1].fault_code,1);
  assert.equal(faultNotices[1].message_id,1);
  assert.equal(faultNotices[1].supervisor_id,21);
  assert.equal(faultNotices[1].generation,workerB.generation);
  assert.ok(notices.every(x=>Object.isFrozen(x)));
  for(const n of notices) {
    const wire=notice_to_wire_json(n);
    const parsed=JSON.parse(wire);
    assert.deepEqual(Object.keys(parsed),[
      'kind','actor_id','generation','supervisor_id','message_id','fault_code','reason']);
    assert.equal(parsed.actor_id,n.actor_id);
  }
  console.log('M9_RUST_WASM_CANONICAL_NOTICE_JSON=PASS');
  console.log('M9_REAL_COMPILED_ATTO_WORKER_MESSAGES=PASS');
  console.log('M9_PRIORITY_SUPERVISOR_STOP_THEN_PANIC=PASS');
  console.log('M9_SATURATED_DATA_DOES_NOT_BLOCK_CONTROL=PASS');
  console.log('M9_ACCEPTED_FAILED_ABANDONED_CONSERVATION=PASS');
  console.log('M9_STALE_ACTOR_AUTHORITY_REJECTED=PASS');
  console.log('M9_WORKER_COOPERATIVE_YIELDS='+a.yielded);
  // Host-triggered PANIC must abandon a genuinely suspended continuation
  // without a second receive/double settlement.
  const c=host.spawn(make_worker(),healthy,21);
  assert.equal(host.try_send(c,50000),'ACCEPTED');
  assert.equal(host.try_send(c,10),'ACCEPTED');
  const deadline=Date.now()+15000;
  while(host.snapshot(c).yielded<3 && Date.now()<deadline)
    await new Promise(resolve=>setTimeout(resolve,2));
  assert.ok(host.snapshot(c).yielded>=3,'worker did not enter suspended state');
  assert.equal(host.request_panic(c,'host cancellation'),true);
  assert.ok(await host.await_terminal(c,15000),'cancelled worker never acknowledged close');
  const cancelled=host.check_conservation(c);
  assert.equal(cancelled.accepted,2);
  assert.equal(cancelled.abandoned+cancelled.failed,2);
  assert.equal(cancelled.completed,0);
  assert.equal(notices.filter(x=>x.actor_id===c.actor_id&&x.kind==='PANIC').length,1);
  assert.equal(host.try_send(c,1),'STALE');
  console.log('M9_SUSPENDED_WORKER_CANCEL_AND_QUIESCENCE=PASS');
} finally {await host.close();}
console.log('M9_SCOPE=DEDICATED_WORKERS_RESTRICTED_COMPILED_ATTO_NOT_POOLED_OR_FULL_ACTOR_HOST');

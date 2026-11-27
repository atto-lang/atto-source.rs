import assert from 'node:assert/strict';
import test from 'node:test';
import {createStateRuntime} from './browser/state-runtime.mjs';

test('host-issued i64 ticket, closed without replay, retirement fenced', () => {
  const events=[], runtime=createStateRuntime({onEvent:e=>events.push(e.kind)});
  const {state_register, state_enter, state_leave}=runtime.imports;
  state_register(0);
  state_register(8);
  const a=state_enter(0,0);
  const b=state_enter(0,0);
  assert.equal(typeof a,'bigint');
  assert.notEqual(a,b);
  assert.equal(runtime.snapshot().active,2);
  assert.throws(()=>state_enter(0,1),/contention/);
  assert.throws(()=>state_enter(8,9),/invalid lease mode/);
  assert.throws(()=>state_enter(16,0),/unknown state container/);
  runtime.beginRetire(0);
  assert.equal(runtime.quiescent(0),false);
  assert.throws(()=>state_enter(0,0),/retired/);
  state_leave(a);
  state_leave(b);
  assert.equal(runtime.quiescent(0),true);
  assert.throws(()=>state_leave(a),/replayed admission ticket/);
  const c=state_enter(8,1);
  state_leave(c);
  runtime.complete();
  assert.deepEqual(runtime.snapshot().active,0);
  assert.deepEqual(events.slice(0,2),['register','register']);
  assert.throws(()=>state_register(24),/after retirement/);
});

test('different guest instances cannot replay another incarnation ticket', () => {
  const left=createStateRuntime(), right=createStateRuntime();
  left.imports.state_register(0);
  right.imports.state_register(0);
  const t=left.imports.state_enter(0,1);
  assert.throws(()=>right.imports.state_leave(t),/replayed admission ticket/);
  assert.notEqual(right.imports.state_enter(0,1),t);
  left.imports.state_leave(t);
  left.complete(); right.abort();
});

test('physical trap unwinding abort clears authority and prevents resume', () => {
  const runtime=createStateRuntime();
  runtime.imports.state_register(0);
  const t=runtime.imports.state_enter(0,1);
  assert.throws(()=>runtime.complete(),/unreleased state tickets/);
  assert.equal(runtime.snapshot().active,1);
  runtime.abort();
  assert.deepEqual(runtime.snapshot().active,0);
  assert.equal(runtime.snapshot().quiescent,true);
  assert.throws(()=>runtime.imports.state_leave(t),/replayed admission ticket/);
  assert.throws(()=>runtime.imports.state_enter(0,0),/retired/);
});

test('bounded instances and forged pointer fail closed', () => {
  const runtime=createStateRuntime({maxContainers:1,maxTickets:1});
  assert.throws(()=>runtime.imports.state_register(-1),/invalid state pointer/);
  assert.throws(()=>runtime.imports.state_register(2),/invalid state pointer/);
  runtime.imports.state_register(0);
  assert.throws(()=>runtime.imports.state_register(0),/pointer reused/);
  assert.throws(()=>runtime.imports.state_register(8),/container admission capacity/);
  const t=runtime.imports.state_enter(0,0);
  assert.throws(()=>runtime.imports.state_enter(0,0),/admission capacity/);
  runtime.imports.state_leave(t);
  runtime.complete();
});


test('throwing observers cannot orphan tickets or interrupt retirement', () => {
  const events=[], frozen=[];
  const runtime=createStateRuntime({onEvent:e=>{
    events.push(e.kind);
    frozen.push(Object.isFrozen(e));
    throw new Error('observer failed');
  }});
  const {state_register,state_enter,state_leave}=runtime.imports;
  state_register(0);
  const ticket=state_enter(0,1);
  assert.equal(typeof ticket,'bigint');
  assert.equal(runtime.snapshot().active,1);
  runtime.beginRetire(0);
  assert.equal(runtime.quiescent(0),false);
  state_leave(ticket);
  assert.equal(runtime.quiescent(0),true);
  runtime.complete();
  assert.equal(runtime.snapshot().quiescent,true);
  assert.deepEqual(events,['register','enter','retire','leave','complete']);
  assert.deepEqual(frozen,[true,true,true,true,true]);
  assert.equal(runtime.observerFailureCount(),5);
});

test('observer cannot reenter any mutating admission operation', () => {
  let runtime, existingTicket;
  let probes=0;
  runtime=createStateRuntime({onEvent:e=>{
    if (e.kind!=='enter' || !existingTicket) return;
    const {state_register,state_enter,state_leave}=runtime.imports;
    for (const action of [
      ()=>state_register(16), ()=>state_enter(8,0),
      ()=>state_leave(existingTicket), ()=>runtime.beginRetire(0),
      ()=>runtime.complete(), ()=>runtime.abort(),
    ]) {
      assert.throws(action,/observer cannot mutate admission/);
      probes++;
    }
    assert.equal(runtime.snapshot().active,2);
  }});
  runtime.imports.state_register(0);
  runtime.imports.state_register(8);
  existingTicket=runtime.imports.state_enter(8,1);
  const ticket=runtime.imports.state_enter(0,1);
  assert.equal(probes,6);
  assert.equal(runtime.observerFailureCount(),0);
  assert.equal(runtime.snapshot().containers,2);
  assert.equal(runtime.snapshot().retired,false);
  runtime.imports.state_leave(ticket);
  runtime.imports.state_leave(existingTicket);
  runtime.complete();
});

test('observer reentry failure is isolated during physical abort', () => {
  let runtime;
  runtime=createStateRuntime({onEvent:e=>{
    if (e.kind==='abort') runtime.imports.state_register(8);
  }});
  runtime.imports.state_register(0);
  const ticket=runtime.imports.state_enter(0,1);
  runtime.abort();
  assert.equal(runtime.observerFailureCount(),1);
  assert.equal(runtime.snapshot().active,0);
  assert.equal(runtime.snapshot().quiescent,true);
  assert.throws(()=>runtime.imports.state_leave(ticket),/replayed/);
});

test('invalid observer rejected at runtime construction', () => {
  assert.throws(()=>createStateRuntime({onEvent:42}),/invalid state observer/);
});

test('independent reader counts, exclusivity and retired generation checks',()=>{
  const rt=createStateRuntime({maxContainers:64,maxTickets:64});
  const {state_register:reg,state_enter:enter,state_leave:leave}=rt.imports;
  for(let i=1;i<=32;i++)reg(i*8);
  const reads=Array.from({length:32},(_,i)=>enter((i+1)*8,0));
  const second=enter(8,0);
  assert.throws(()=>enter(8,1),/contention/);
  leave(reads[0]);
  assert.throws(()=>enter(8,1),/contention/);
  leave(second);
  const w=enter(8,1);
  assert.throws(()=>enter(8,0),/contention/);
  assert.throws(()=>enter(8,1),/contention/);
  rt.beginRetire(8);
  assert.equal(rt.quiescent(8),false);
  leave(w);
  assert.equal(rt.quiescent(8),true);
  assert.throws(()=>leave(w),/replayed/);
  assert.throws(()=>enter(8,0),/retired/);
  for(const ticket of reads.slice(1))leave(ticket);
  rt.complete();
  assert.equal(rt.snapshot().active,0);
  assert.equal(rt.snapshot().quiescent,true);
});
test('abort clears per-container counters, stale tickets and foreign authority',()=>{
  const a=createStateRuntime(),b=createStateRuntime();
  a.imports.state_register(8);a.imports.state_register(16);b.imports.state_register(8);
  const r=a.imports.state_enter(8,0),w=a.imports.state_enter(16,1);
  const foreign=b.imports.state_enter(8,1);
  assert.throws(()=>a.imports.state_leave(foreign),/missing or replayed/);
  assert.throws(()=>a.complete(),/unreleased/);
  a.abort();
  for(const ptr of [8,16])assert.equal(a.quiescent(ptr),true);
  for(const ticket of [r,w])assert.throws(()=>a.imports.state_leave(ticket),/replayed/);
  assert.equal(a.snapshot().active,0);
  assert.equal(b.snapshot().active,1);
  b.imports.state_leave(foreign);b.complete();
});
test('bounded deterministic differential ledger under mixed admission and release',()=>{
  const rt=createStateRuntime({maxContainers:16,maxTickets:64});
  const model=Array.from({length:16},()=>({r:0,w:false}));
  const tickets=[];
  for(let i=0;i<16;i++)rt.imports.state_register(i*8);
  let rng=123456789;
  const rand=()=>{rng=(Math.imul(rng,1664525)+1013904223)>>>0;return rng;};
  for(let step=0;step<10000;step++){
    if(tickets.length&&(rand()>>>16)%3===0){
      const index=(rand()>>>8)%tickets.length;
      const [{ticket,i,mode}]=tickets.splice(index,1);
      rt.imports.state_leave(ticket);
      if(mode)model[i].w=false;else model[i].r--;
    }else{
      const i=(rand()>>>8)%16,mode=(rand()>>>16)%2,m=model[i];
      if(tickets.length===64||m.w||(mode&&m.r)){
        assert.throws(()=>rt.imports.state_enter(i*8,mode),/capacity|contention/);
      }else{
        tickets.push({ticket:rt.imports.state_enter(i*8,mode),i,mode});
        if(mode)m.w=true;else m.r++;
      }
    }
    assert.equal(rt.snapshot().active,tickets.length);
  }
  for(const {ticket} of tickets)rt.imports.state_leave(ticket);
  rt.complete();assert.equal(rt.snapshot().quiescent,true);
});

test('state pointer domain is bounded by the signed i32 preview ABI', () => {
  const rt=createStateRuntime({maxContainers:2,maxTickets:1});
  const {state_register:reg,state_enter:enter,state_leave:leave}=rt.imports;
  const before=rt.snapshot();
  for(const ptr of [0x80000000,0xfffffff8,0x100000000,2**53,Infinity,NaN,-8,1.5,'8',8n]){
    assert.throws(()=>reg(ptr),/invalid state pointer/);
    assert.throws(()=>enter(ptr,0),/unknown state container/);
    assert.throws(()=>rt.beginRetire(ptr),/unknown state container/);
    assert.throws(()=>rt.quiescent(ptr),/unknown state container/);
    assert.deepEqual(rt.snapshot(),before);
  }
  reg(0);reg(0x7ffffff8);
  const ticket=enter(0x7ffffff8,1);
  assert.equal(rt.snapshot().active,1);
  assert.throws(()=>enter(0x100000000,1),/unknown state container/);
  assert.equal(rt.snapshot().active,1);
  leave(ticket);rt.complete();
  assert.equal(rt.snapshot().quiescent,true);
});

test('retire and successful completion are one-shot notifications',()=>{
  const events=[];const rt=createStateRuntime({onEvent:e=>events.push(e.kind)});
  rt.imports.state_register(0); const t=rt.imports.state_enter(0,0);
  rt.beginRetire(0);rt.beginRetire(0);
  assert.equal(rt.quiescent(0),false);
  rt.imports.state_leave(t);rt.complete();rt.complete();rt.abort();
  assert.deepEqual(events,['register','enter','retire','leave','complete']);
  assert.equal(rt.snapshot().quiescent,true);
});
test('aborted runtime never reports successful completion',()=>{
  const events=[];const rt=createStateRuntime({onEvent:e=>events.push(e.kind)});
  rt.imports.state_register(0);const t=rt.imports.state_enter(0,1);
  rt.abort();rt.abort();assert.throws(()=>rt.complete(),/aborted/);
  assert.throws(()=>rt.imports.state_leave(t),/replayed/);
  assert.deepEqual(events,['register','enter','abort']);
  assert.equal(rt.quiescent(0),true);
});
test('failed complete does not terminalize live runtime',()=>{
  const events=[];const rt=createStateRuntime({onEvent:e=>events.push(e.kind)});
  rt.imports.state_register(0);const t=rt.imports.state_enter(0,1);
  assert.throws(()=>rt.complete(),/unreleased/);assert.equal(rt.snapshot().retired,false);
  rt.imports.state_leave(t);rt.complete();assert.equal(rt.snapshot().quiescent,true);
  assert.deepEqual(events,['register','enter','leave','complete']);
});

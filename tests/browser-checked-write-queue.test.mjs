import test from 'node:test';
import assert from 'node:assert/strict';
import {CheckedWriteQueue} from '../runtime/browser/checked-write-queue.mjs';

const afterTurn=()=>new Promise(resolve=>setImmediate(resolve));
const gate=()=>{let resolve;const promise=new Promise(r=>{resolve=r});return{promise,resolve}};
const okSink=async bytes=>({ok:true,written:bytes.length});
const submit=(q,payload,fields={})=>q.submitWrite({owner:'actor-1',generation:1,payload,...fields});

test('nb submit returns promptly, copies bytes and resolves a checked write result',async()=>{
  const release=gate();const observed=[];
  const q=new CheckedWriteQueue({slots:2,maxBytes:64,sink:async b=>{
    observed.push([...b]);await release.promise;return{ok:true,written:b.length};
  }});
  const raw=new Uint8Array([104,105]);
  const accepted=submit(q,raw);
  assert.equal(accepted.ok,true);
  assert.equal(q.stats.pending,1);
  raw[0]=0; // caller mutation cannot alter submitted payload
  await afterTurn();
  assert.deepEqual(observed,[[104,105]]);
  assert.equal(q.stats.committed,true);
  release.resolve();
  assert.deepEqual(await accepted.future,{ok:true,written:2});
  assert.equal(q.stats.pending,0);
  assert.equal(q.stats.bytes,0);
});

test('credit and bytes quotas reject synchronously retaining the original payload',async()=>{
  const blocked=gate();
  const q=new CheckedWriteQueue({slots:1,maxBytes:3,sink:async b=>{
    await blocked.promise;return{ok:true,written:b.length};
  }});
  const first=submit(q,'abc');
  const same={id:'owned'};
  const second=submit(q,same.id);
  assert.equal(second.ok,false);
  assert.equal(second.error.code,'E_IO_QUOTA');
  assert.equal(second.payload,same.id);
  blocked.resolve();
  assert.equal((await first.future).ok,true);
  await afterTurn();
  assert.equal(q.stats.pending,0);
});

test('byte quota is the sum of queued and committed ownership',async()=>{
  const blocked=gate();
  const q=new CheckedWriteQueue({slots:3,maxBytes:4,sink:async b=>{
    await blocked.promise;return{ok:true,written:b.length};
  }});
  const one=submit(q,'abc');await afterTurn();
  const bad=submit(q,'de');
  assert.equal(bad.error.code,'E_IO_QUOTA');
  assert.equal(q.stats.bytes,3);
  blocked.resolve();await one.future;await afterTurn();
  assert.equal(submit(q,'de').ok,true);
});

test('FIFO ordering is enforced under async sink backpressure',async()=>{
  const calls=[];const release=gate();
  const q=new CheckedWriteQueue({slots:4,maxBytes:32,sink:async b=>{
    const msg=new TextDecoder().decode(b);
    calls.push(msg);
    if(msg==='one')await release.promise;
    return{ok:true,written:b.length};
  }});
  const a=submit(q,'one'),b=submit(q,'two'),c=submit(q,'three');
  await afterTurn();assert.deepEqual(calls,['one']);
  release.resolve();
  const outcomes=await Promise.all([a.future,b.future,c.future]);
  assert.deepEqual(calls,['one','two','three']);
  assert(outcomes.every(r=>r.ok));
});

test('cancel before commit never sends queued payload and is exactly-once',async()=>{
  const release=gate(),delivered=[];
  const q=new CheckedWriteQueue({slots:2,maxBytes:30,sink:async b=>{
    const msg=new TextDecoder().decode(b);
    delivered.push(msg);
    if(msg==='first')await release.promise;
    return{ok:true,written:b.length};
  }});
  const first=submit(q,'first');await afterTurn();
  const second=submit(q,'second');
  assert.equal(q.cancel(second.ticket),true);
  assert.equal(q.cancel(second.ticket),false);
  assert.deepEqual(await second.future,{ok:false,error:{code:'E_IO_CANCELLED',
    message:'write cancelled before commit'}});
  release.resolve();await first.future;await afterTurn();
  assert.deepEqual(delivered,['first']);
});

test('cancel after commit returns false and cannot pretend to roll back external effects',async()=>{
  const release=gate(),delivered=[];
  const q=new CheckedWriteQueue({slots:1,maxBytes:10,sink:async b=>{
    delivered.push(new TextDecoder().decode(b));
    await release.promise;
    return{ok:true,written:b.length};
  }});
  const value=submit(q,'sent');await afterTurn();
  assert.equal(q.cancel(value.ticket),false);
  release.resolve();
  assert.equal((await value.future).ok,true);
  assert.deepEqual(delivered,['sent']);
});

test('stopping an owner cancels queued messages without running callbacks',async()=>{
  const release=gate(),observed=[],supervisor=[],callbacks=[];
  const q=new CheckedWriteQueue({slots:3,maxBytes:30,supervisor:e=>supervisor.push(e),
    sink:async b=>{observed.push(new TextDecoder().decode(b));await release.promise;return{ok:true,written:b.length}}});
  const a=submit(q,'first',{mode:'do',onOutcome:r=>callbacks.push(r)});
  await afterTurn();
  const b=submit(q,'second',{mode:'do',onOutcome:r=>callbacks.push(r)});
  assert.equal(a.ok,true);assert.equal(b.ok,true);
  q.stopOwner('actor-1',1);
  assert.equal(q.stats.pending,1); // in-flight commit is irreversible
  assert.equal(supervisor.length,1);
  assert.equal(supervisor[0].outcome.error.code,'E_IO_CANCELLED');
  release.resolve();await afterTurn();await afterTurn();
  assert.deepEqual(observed,['first']);
  assert.deepEqual(callbacks,[]);
  assert.equal(q.stats.pending,0);
});

test('stopped generation rejects writes while a new owner generation can proceed',async()=>{
  const q=new CheckedWriteQueue({sink:okSink});
  q.stopOwner('actor-1',7);
  const rejected=submit(q,'old',{generation:7});
  assert.equal(rejected.error.code,'E_IO_OWNER');
  const fresh=submit(q,'new',{generation:8});
  assert.equal(fresh.ok,true);assert.equal((await fresh.future).ok,true);
});

test('a bare do write succeeds without callback and must have supervisor',async()=>{
  const qNoSupervisor=new CheckedWriteQueue({sink:okSink});
  const bad=submit(qNoSupervisor,'a',{mode:'do'});
  assert.equal(bad.error.code,'E_IO_SUPERVISOR');
  const events=[];
  const q=new CheckedWriteQueue({sink:okSink,supervisor:e=>events.push(e)});
  const a=submit(q,'hello',{mode:'do'});
  assert.equal(a.ok,true);
  assert.equal('future' in a,false);
  await afterTurn();await afterTurn();
  assert.deepEqual(events,[]);
  assert.equal(q.stats.pending,0);
});

test('detached errors with no callback reach supervisor exactly once',async()=>{
  const events=[];
  const q=new CheckedWriteQueue({sink:async()=>({ok:false,error:{code:'E_IO_CLOSED',message:'closed channel'}}),
    supervisor:e=>events.push(e)});
  submit(q,'hello',{mode:'do'});
  await afterTurn();await afterTurn();
  assert.equal(events.length,1);
  assert.equal(events[0].outcome.error.code,'E_IO_CLOSED');
});

test('typed detached callback receives both checked success and failure once',async()=>{
  const outcomes=[],notices=[];
  let n=0;
  const q=new CheckedWriteQueue({sink:async b=>++n===1
    ?{ok:true,written:b.length}:{ok:false,error:{code:'E_IO_CLOSED',message:'closed'}},
    supervisor:e=>notices.push(e)});
  submit(q,'success',{mode:'do',onOutcome:r=>outcomes.push(r)});
  submit(q,'failure',{mode:'do',onOutcome:r=>outcomes.push(r)});
  await afterTurn();await afterTurn();
  assert.deepEqual(outcomes.map(x=>x.ok),[true,false]);
  assert.deepEqual(notices,[]);
});

test('success-only callback runs only on successful writes, errors still supervised',async()=>{
  const success=[],errors=[];let n=0;
  const q=new CheckedWriteQueue({sink:async b=>++n===1
    ?{ok:true,written:b.length}:{ok:false,error:{code:'E_IO_ADAPTER',message:'fail'}},
    supervisor:e=>errors.push(e)});
  submit(q,'ok',{mode:'do',onSuccess:()=>success.push('done')});
  submit(q,'fail',{mode:'do',onSuccess:()=>success.push('wrong')});
  await afterTurn();await afterTurn();
  assert.deepEqual(success,['done']);
  assert.deepEqual(errors.map(e=>e.outcome.error.code),['E_IO_ADAPTER']);
});

test('failure to acknowledge every byte is a checked adapter error, not success',async()=>{
  const q=new CheckedWriteQueue({sink:async b=>({ok:true,written:b.length-1})});
  const r=submit(q,'hello');
  assert.equal((await r.future).error.code,'E_IO_ADAPTER');
});

test('adapter exceptions are reported as checked failures, not unhandled rejections',async()=>{
  const q=new CheckedWriteQueue({sink:async()=>{throw new Error('broken adapter')}});
  const r=submit(q,'a');
  const result=await r.future;
  assert.equal(result.error.code,'E_IO_ADAPTER');
  assert.match(result.error.message,/broken adapter/);
});

test('closing the writer cancels queued work, prevents further submissions and releases quotas',async()=>{
  const gate1=gate(),emitted=[];
  const q=new CheckedWriteQueue({slots:2,maxBytes:20,sink:async b=>{
    emitted.push(new TextDecoder().decode(b));await gate1.promise;return{ok:true,written:b.length};
  }});
  const a=submit(q,'in-flight');await afterTurn();
  const b=submit(q,'queued');
  q.close();q.close();
  assert.equal((await b.future).error.code,'E_IO_CLOSED');
  assert.equal(submit(q,'new').error.code,'E_IO_CLOSED');
  gate1.resolve();assert.equal((await a.future).ok,true);
  await afterTurn();
  assert.equal(q.stats.pending,0);
  assert.equal(q.stats.bytes,0);
  assert.deepEqual(emitted,['in-flight']);
});

test('repeated cancel/re-admit while one committed write stalls cannot grow FIFO',async()=>{
  const release=gate();
  const q=new CheckedWriteQueue({slots:2,maxBytes:10,sink:async b=>{
    await release.promise;return{ok:true,written:b.length};
  }});
  const first=submit(q,'x');await afterTurn();
  for(let n=0;n<1000;n++){
    const t=submit(q,'z');assert.equal(t.ok,true);
    assert.equal(q.cancel(t.ticket),true);
    assert.equal((await t.future).error.code,'E_IO_CANCELLED');
    assert.equal(q.stats.queued,0);
  }
  assert.equal(q._fifo.length,0);
  release.resolve();await first.future;
});

test('callback errors and rejected async callbacks are supervised, not silent',async()=>{
  const notices=[];
  const q=new CheckedWriteQueue({sink:okSink,supervisor:e=>notices.push(e)});
  submit(q,'sync',{mode:'do',onSuccess:()=>{throw new Error('callback exploded')}});
  submit(q,'async',{mode:'do',onOutcome:async()=>{throw new Error('async callback exploded')}});
  await afterTurn();await afterTurn();
  assert(notices.length>=2);
  assert(notices.every(e=>e.outcome.error.code==='E_IO_CALLBACK'));
});

test('supervisor failure is retained and visible to its owning Worker controller',async()=>{
  const q=new CheckedWriteQueue({sink:async()=>({ok:false,error:{code:'E_IO_CLOSED'}}),
    supervisor:()=>{throw new Error('supervisor unavailable')}});
  submit(q,'failed',{mode:'do'});
  await afterTurn();await afterTurn();
  const failures=q.takeSupervisorFailures();
  assert.equal(failures.length,1);
  assert.match(String(failures[0]),/supervisor unavailable/);
  assert.equal(q.stats.supervisorFailures,0);
});

test('Unicode payload byte size is measured in UTF-8, not UTF-16 characters',async()=>{
  const q=new CheckedWriteQueue({slots:1,maxBytes:3,sink:okSink});
  const r=submit(q,'á'); // two UTF-8 bytes
  assert.equal(r.ok,true);
  assert.deepEqual(await r.future,{ok:true,written:2});
  assert.equal(submit(q,'💡').error.code,'E_IO_QUOTA'); // four bytes
});

test('invalid callback and owner combinations fail synchronously without registration',()=>{
  const q=new CheckedWriteQueue({sink:okSink,supervisor:()=>{}});
  for(const fields of [
    {owner:'',generation:1,payload:'x'},
    {owner:'actor-1',generation:-1,payload:'x'},
    {owner:'actor-1',generation:1,payload:'x',mode:'illegal'},
    {owner:'actor-1',generation:1,payload:'x',mode:'nb',onOutcome:()=>{}},
    {owner:'actor-1',generation:1,payload:'x',mode:'do',onOutcome:()=>{},onSuccess:()=>{}},
  ]){
    assert.equal(q.submitWrite(fields).error.code,'E_IO_SYNTAX');
  }
  assert.equal(q.stats.pending,0);
});

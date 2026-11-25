import test from 'node:test';
import assert from 'node:assert/strict';
import {
  BrowserRunSession, RUN_DEADLINE_MS,
  MAX_RUN_OUTPUT_CHARS, MAX_RUN_ERROR_CHARS,
} from './browser/run-watchdog.mjs';

const delay=ms=>new Promise(resolve=>setTimeout(resolve,ms));
class FakeWorker {
  constructor() {
    this.messages=[];
    this.terminations=0;
  }
  postMessage(data) { this.messages.push(data); }
  terminate() { this.terminations++; }
  emit(data) { this.onmessage?.({data}); }
}
const ok = {ok:true,value:'42',stdout:'ready\n'};
function begin(worker=new FakeWorker(),config={}) {
  const received=[];
  const session=new BrowserRunSession(worker,{source:'test'},r=>received.push(r),config);
  return {worker,session,received};
}
test('real timeout terminates an otherwise unresponsive child once',async()=>{
  const {worker,session,received}=begin(new FakeWorker(),{deadline_ms:25});
  await delay(42);
  assert.equal(session.state,'completed');
  assert.deepEqual(received,[{ok:false,error:'E_TIMEOUT'}]);
  assert.equal(worker.terminations,1);
});

test('an outdated success event cannot overwrite the deadline outcome',async()=>{
  const {worker,session,received}=begin(new FakeWorker(),{deadline_ms:25});
  const stale=worker.onmessage;
  await delay(42);
  stale({data:ok});
  worker.emit(ok);
  assert.equal(session.finish(ok),false);
  assert.deepEqual(received,[{ok:false,error:'E_TIMEOUT'}]);
  assert.equal(worker.terminations,1);
});

test('a successful response clears timer and survives stale timeout attempts',async()=>{
  const {worker,session,received}=begin(new FakeWorker(),{deadline_ms:30});
  worker.emit(ok);
  await delay(45);
  assert.deepEqual(received,[ok]);
  assert.equal(session.state,'completed');
  assert.equal(session.finish({ok:false,error:'E_TIMEOUT'}),false);
  assert.equal(worker.terminations,1);
});

test('worker errors, clone failures and failed postMessage clean up exactly once',()=>{
  for(const [kind,expected] of [
    ['error','E_WORKER'],['messageerror','E_WORKER_MESSAGE'],
    ['clone','E_WORKER_CLONE'],
  ]) {
    const worker=new FakeWorker();
    if(kind==='clone')worker.postMessage=()=>{throw new Error('cloning failed')};
    const {session,received}=begin(worker);
    if(kind==='error')worker.onerror({message:'boom'});
    if(kind==='messageerror')worker.onmessageerror({message:'boom'});
    assert.deepEqual(received,[{ok:false,error:expected}]);
    assert.equal(session.state,'completed');
    assert.equal(worker.terminations,1);
  }
});

test('cancellation forbids late success, including a captured onmessage callback',()=>{
  const {worker,session,received}=begin();
  const queued=worker.onmessage;
  assert.equal(session.cancel(),true);
  queued({data:ok});
  assert.equal(session.cancel(),false);
  assert.deepEqual(received,[{ok:false,error:'E_CANCELLED'}]);
  assert.equal(worker.terminations,1);
});

test('malformed worker messages fail closed and cannot certify main completion',()=>{
  for(const response of [null,undefined,{},[],true,42,
    {ok:true,value:42,stdout:''},
    {ok:true,value:'42',stdout:42},
    {ok:'true',value:'42',stdout:''},
    {ok:false,error:null},
    {ok:false,error:'X'.repeat(MAX_RUN_ERROR_CHARS+1)},
    {ok:true,value:'0'.repeat(41),stdout:''},
    {ok:true,value:'42',stdout:'a'.repeat(MAX_RUN_OUTPUT_CHARS+1)},
  ]) {
    const {worker,received}=begin();
    worker.emit(response);
    assert.deepEqual(received,[{ok:false,error:'E_WORKER_PROTOCOL'}]);
    assert.equal(worker.terminations,1);
  }
});

test('exact output quota and longest valid error pass without truncation',()=>{
  const {worker,received}=begin();
  const output='z'.repeat(MAX_RUN_OUTPUT_CHARS);
  worker.emit({ok:true,value:'-1',stdout:output});
  assert.deepEqual(received,[{ok:true,value:'-1',stdout:output}]);
  const second=begin();
  second.worker.emit({ok:false,error:'e'.repeat(MAX_RUN_ERROR_CHARS)});
  assert.equal(second.received[0].error.length,MAX_RUN_ERROR_CHARS);
});

test('session rejects invalid deadlines and nonworker objects',()=>{
  for(const timeout of [0,-1,1.5,NaN,Infinity,RUN_DEADLINE_MS+1]) {
    assert.throws(
      ()=>begin(new FakeWorker(),{deadline_ms:timeout}),
      /E_RUN_SESSION_ARGS/,
    );
  }
  assert.throws(
    ()=>new BrowserRunSession(null,{},()=>{}),/E_RUN_SESSION_ARGS/);
  assert.throws(
    ()=>new BrowserRunSession({}, {}, ()=>{}),/E_RUN_SESSION_ARGS/);
  assert.throws(
    ()=>new BrowserRunSession(new FakeWorker(),{},null),/E_RUN_SESSION_ARGS/);
});

test('success and failure are isolated across unrelated run sessions',()=>{
  const a=begin(),b=begin(),c=begin();
  a.worker.emit(ok);
  b.session.cancel();
  c.worker.onerror();
  a.worker.emit({ok:false,error:'E_TIMEOUT'});
  assert.deepEqual(a.received,[ok]);
  assert.deepEqual(b.received,[{ok:false,error:'E_CANCELLED'}]);
  assert.deepEqual(c.received,[{ok:false,error:'E_WORKER'}]);
  for(const x of [a,b,c])assert.equal(x.worker.terminations,1);
});

import test from 'node:test';
import assert from 'node:assert/strict';
import {readFileSync,mkdtempSync,rmSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {join,resolve} from 'node:path';
import {pathToFileURL} from 'node:url';
import {spawnSync} from 'node:child_process';
import {Worker} from 'node:worker_threads';
import {instantiateOwnedUnwind,startOwnedUnwind} from '../runtime/browser/owned-unwind-host-v1.mjs';
const js=process.env.ATTO_JS_SOURCE;
if(!js)throw Error('ATTO_JS_SOURCE must point to exact JS backend source');
const importJs=p=>import(pathToFileURL(resolve(js,'src',p)).href);
const {createWorkerControlLoop}=await importJs('worker-control-loop.mjs');
const {SelectableI64Future}=await importJs('selectable-i64-future.mjs');
const runtime=await importJs('scheduled-unwind-runtime.mjs');
const dir=mkdtempSync(join(tmpdir(),'atto-rust-recover-'));
let bytes;
try{
 const file=join(dir,'recover.wasm');
 const p=spawnSync('target/debug/attoc',['--owned-unwind-v1','tests/fixtures/owned-pending-recover.atto','-o',file],{encoding:'utf8'});
 assert.equal(p.status,0,p.stderr);bytes=readFileSync(file);
}finally{rmSync(dir,{recursive:true,force:true});}
function setup(options={}){
 const turns=[],errors=[],out=[];
 const scheduler=createWorkerControlLoop({...options,postTurn:f=>turns.push(f),onTurnError:e=>errors.push(e)});
 scheduler.attachOwner('a',1);scheduler.attachOwner('b',1);
 const futures=Array.from({length:3},()=>new SelectableI64Future());
 const instance=instantiateOwnedUnwind(new WebAssembly.Module(bytes));
 const start=(extra={})=>startOwnedUnwind({instance,scheduler,runtime,ownerKey:'a',owner:7,epoch:1,futures,onDone:v=>{assert.equal(scheduler.retireOwner('a',1),false);out.push(v);},...extra});
 return {scheduler,futures,instance,start,out,turns,step(){assert.ok(turns.length);turns.shift()();assert.deepEqual(errors,[]);}};
}
test('real Rust Wasm with exact JS scheduler and Future implementation',()=>{
 const h=setup({maxSuspended:1}),[input,cleanup,fallback]=h.futures,err=Error('original');
 assert.equal(h.start(),true);assert.equal(h.scheduler.retireOwner('a',1),false);h.step();
 assert.equal(input.pendingWaiters,1);assert.equal(cleanup.pendingWaiters,0);
 input.fail(err);assert.equal(cleanup.pendingWaiters,0);h.step();assert.equal(cleanup.pendingWaiters,1);
 cleanup.resolve(0n);assert.equal(fallback.pendingWaiters,0);h.step();assert.equal(fallback.pendingWaiters,1);
 fallback.resolve(42n);assert.deepEqual(h.out,[]);h.step();
 assert.deepEqual(h.out,[{kind:'recovered',value:42n,error:err}]);assert.equal(h.scheduler.stats.retainedOwners,0);assert.equal(h.scheduler.retireOwner('a',1),true);
});
test('atomic claims deny reuse; ready queue failure refunds all',()=>{
 const h=setup({maxReady:1});h.scheduler.admitData('a',1,()=>{});
 assert.equal(h.start(),false);assert.equal(h.scheduler.stats.retainedOwners,0);h.step();
 assert.equal(h.start(),true);
 assert.throws(()=>h.start(),/INSTANCE_USED/);
 const other=instantiateOwnedUnwind(new WebAssembly.Module(bytes));
 assert.throws(()=>h.start({instance:other,ownerKey:'b',owner:8}),/FUTURE_MOVED/);
});
test('real Node Worker executes Rust compiled guest under exact shared scheduler',async()=>{
 const worker=new Worker(new URL('./fixtures/owned-unwind-node-worker.mjs',import.meta.url),{workerData:{bytes,js:resolve(js)}});
 const messages=[];let wake;
 worker.on('message',m=>{messages.push(m);wake?.();});
 async function next(kind){
  for(let i=0;i<20;i++){
   const err=messages.find(m=>m.kind==='error');if(err)throw Error(err.message);
   const m=messages.find(m=>m.kind===kind);if(m)return m;
   await new Promise((ok,fail)=>{const timer=setTimeout(()=>{wake=null;fail(Error('timeout '+kind));},1000);wake=()=>{clearTimeout(timer);wake=null;ok();};});
  }throw Error('worker wait bound');
 }
 try{
  await next('body');worker.postMessage('fail');await next('cleanup');
  worker.postMessage('clean');await next('recover');worker.postMessage('fallback');
  assert.deepEqual(await next('done'),{kind:'done',value:'42',sameError:true});
  worker.postMessage('retire');await next('retired');
  assert.deepEqual(messages.map(m=>m.kind),['body','cleanup','recover','done','retired']);
 }finally{await worker.terminate();}
});

test('compiler-pinned arity and instance provenance reject before claim',()=>{
 const h=setup();
 assert.throws(()=>h.start({futures:h.futures.slice(0,2)}),/ARITY/);
 assert.throws(()=>h.start({futures:[...h.futures,new SelectableI64Future()]}),/ARITY/);
 assert.throws(()=>h.start({instance:new WebAssembly.Instance(new WebAssembly.Module(bytes))}),/INSTANCE_USED/);
 assert.equal(h.scheduler.stats.retainedOwners,0);assert.equal(h.start(),true);
});
test('JS and Rust guests share the same exclusive Future claim domain',async()=>{
 const {compileScheduledUnwindV1}=await importJs('scheduled-unwind-compiler.mjs');
 const source=readFileSync('tests/fixtures/owned-pending-recover.atto','utf8');
 const guest=await import('data:text/javascript,'+encodeURIComponent(compileScheduledUnwindV1(source)));
 const h=setup();assert.equal(h.start(),true);
 const identity={tenant_id:'test',actor_id:8,generation:1};
 assert.throws(()=>guest.start(runtime,h.scheduler,'b',identity,h.futures,()=>{}),/FUTURE_MOVED/);
 const other=setup();
 assert.equal(guest.start(runtime,other.scheduler,'b',identity,other.futures,()=>{}),true);
 assert.throws(()=>other.start(),/FUTURE_MOVED/);
});

import test from 'node:test';
import assert from 'node:assert/strict';
import {readFileSync,mkdtempSync,rmSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {join} from 'node:path';
import {spawnSync} from 'node:child_process';
const cli=process.env.ATTOC??'target/debug/attoc';
const dir=mkdtempSync(join(tmpdir(),'atto-owned-unwind-'));
const modules={};
try{
 for(const key of ['defer','recover']){
  const file=join(dir,key+'.wasm');
  const p=spawnSync(cli,['--owned-unwind-v1','tests/fixtures/owned-pending-'+key+'.atto','-o',file],{encoding:'utf8'});
  assert.equal(p.status,0,p.stderr);
  modules[key]=new WebAssembly.Module(readFileSync(file));
  assert.deepEqual(WebAssembly.Module.imports(modules[key]),[]);
  assert.ok(WebAssembly.Module.exports(modules[key]).every(e=>e.kind==='function'));
  assert.deepEqual([...new Uint8Array(WebAssembly.Module.customSections(modules[key],'atto.browser.owned_unwind.v1')[0])],[1]);
 }
}finally{rmSync(dir,{recursive:true,force:true});}
function guest(key){
 const e=new WebAssembly.Instance(modules[key]).exports;
 const call=(n,...a)=>e['unwind_'+n+'_v1'](7,1,...a);
 assert.equal(call('start'),1);assert.equal(call('start'),-1);
 return {e,call,pending:()=>call('pending'),complete:(slot,kind,value)=>call('complete',slot,kind,value),resume:()=>call('resume')};
}
function wake(g,slot,kind,value,next){
 assert.equal(g.pending(),slot);assert.equal(g.resume(),-1);assert.equal(g.call('retire'),-1);
 assert.equal(g.e.unwind_complete_v1(8,1,slot,kind,value),-1);
 assert.equal(g.e.unwind_complete_v1(7,2,slot,kind,value),-1);
 assert.equal(g.complete(slot===1?2:1,kind,value),-1);
 assert.equal(g.complete(slot,3,value),-1);
 assert.equal(g.complete(slot,1,0n),-1);
 assert.equal(g.complete(slot,kind,value),0);assert.equal(g.complete(slot,kind,value),-1);
 assert.equal(g.pending(),-1);assert.throws(()=>g.call('result'),WebAssembly.RuntimeError);
 assert.equal(g.resume(),next);
}
function finish(g,kind,value,error=0n,suppressed=[]){
 assert.equal(g.call('outcome'),kind);
 if(kind<=2)assert.equal(g.call('result'),value);else assert.throws(()=>g.call('result'),WebAssembly.RuntimeError);
 assert.equal(g.call('error'),error);
 assert.equal(g.call('suppressed_count'),suppressed.length);
 for(let i=0;i<suppressed.length;i++){assert.equal(g.call('suppressed_error',i),suppressed[i][0]);assert.equal(g.call('suppressed_rank',i),suppressed[i][1]);}
 assert.throws(()=>g.call('suppressed_error',-1),WebAssembly.RuntimeError);
 assert.throws(()=>g.call('suppressed_error',suppressed.length),WebAssembly.RuntimeError);
 assert.equal(g.call('retire'),0);assert.equal(g.call('retire'),-1);assert.equal(g.call('start'),-1);
 assert.equal(g.complete(1,0,99n),-1);assert.throws(()=>g.call('outcome'),WebAssembly.RuntimeError);
}
test('LIFO pending defer preserves return snapshot',()=>{const g=guest('defer');wake(g,2,0,7n,1);wake(g,1,0,9n,2);finish(g,1,42n);});
test('body failure, cleanup, recovery retain original opaque error token',()=>{const g=guest('recover');wake(g,1,1,11n,1);wake(g,2,0,0n,1);wake(g,3,0,42n,2);finish(g,2,42n,11n);});
test('body success skips fallback',()=>{const g=guest('recover');wake(g,1,0,-42n,1);wake(g,2,0,0n,2);finish(g,1,-42n);});
test('cancel bypasses recovery but executes cleanup',()=>{const g=guest('recover');wake(g,1,2,11n,1);wake(g,2,0,0n,2);finish(g,5,0n,11n);});
test('cleanup failure becomes panic and preserves body failure',()=>{const g=guest('recover');wake(g,1,1,11n,1);wake(g,2,1,22n,2);finish(g,4,0n,22n,[[11n,1]]);});
test('recovery failure becomes panic and preserves body failure',()=>{const g=guest('recover');wake(g,1,1,11n,1);wake(g,2,0,0n,1);wake(g,3,1,33n,2);finish(g,4,0n,33n,[[11n,1]]);});
test('cancellation dominates panic without overwriting original cancellation',()=>{const g=guest('defer');wake(g,2,2,11n,1);wake(g,1,1,22n,2);finish(g,5,0n,11n,[[22n,2]]);});

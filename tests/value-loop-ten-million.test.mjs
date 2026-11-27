import test from 'node:test';
import assert from 'node:assert/strict';
import {spawnSync} from 'node:child_process';
import {Worker} from 'node:worker_threads';
import {resolve} from 'node:path';
import {readFileSync} from 'node:fs';
import {createHash} from 'node:crypto';
const compiler=resolve(process.env.ATTOC??'target/release/attoc');
const source=new URL('fixtures/value_loop_bench/ten-million.atto',import.meta.url);
function compile(){
 const r=spawnSync(compiler,[source.pathname],{timeout:30000,maxBuffer:4*1024*1024});
 assert.ifError(r.error);assert.equal(r.status,0,String(r.stderr));assert.equal(r.signal,null);
 assert.ok(WebAssembly.validate(r.stdout));return r.stdout;
}
async function execute(bytes){
 const worker=new Worker(new URL(import.meta.url),{workerData:bytes});
 try{
  return await new Promise((ok,fail)=>{
   const timer=setTimeout(()=>fail(new Error('10M loop exceeded 20s worker deadline')),20000);
   worker.once('message',r=>{clearTimeout(timer);ok(r)});
   worker.once('error',e=>{clearTimeout(timer);fail(e)});
   worker.once('exit',code=>{clearTimeout(timer);fail(new Error('worker exited before receipt: '+code))});
  });
 }finally{await worker.terminate();}
}
import {isMainThread,parentPort,workerData} from 'node:worker_threads';
if(!isMainThread){
 const module=new WebAssembly.Module(workerData);
 let calls=0;
 const imports=Object.create(null);
 for(const imp of WebAssembly.Module.imports(module)){
  assert.equal(imp.kind,'function','unexpected non-function import');
  (imports[imp.module]??=Object.create(null))[imp.name]=()=>{calls++;throw new Error('unexpected host call '+imp.name)};
 }
 const instance=new WebAssembly.Instance(module,imports);
 const memory=instance.exports.memory;
 const before=memory instanceof WebAssembly.Memory?memory.buffer.byteLength:0;
 const start=performance.now();
 const values=[instance.exports.main(),instance.exports.main()];
 const elapsed=performance.now()-start;
 const after=memory instanceof WebAssembly.Memory?memory.buffer.byteLength:0;
 parentPort.postMessage({values,before,after,calls,elapsed,panic:instance.exports.panic_code.value});
}else{
 test('actual 10M state-loop iterations twice, full i64 result, no host calls or memory growth',async()=>{
  const bytes=compile();
  const receipt=await execute(bytes);
  assert.deepEqual(receipt.values,[49999995000000n,49999995000000n]);
  assert.equal(receipt.before,receipt.after);
  assert.equal(receipt.calls,0);
  assert.equal(receipt.panic,0);
  console.log(JSON.stringify({scope:'bounded-stack execution and no linear-memory growth, not allocation proof',source_sha256:createHash('sha256').update(readFileSync(source)).digest('hex'),wasm_sha256:createHash('sha256').update(bytes).digest('hex'),iterations:20000000,elapsed_ms:receipt.elapsed,memory_bytes:receipt.after,result:'49999995000000'}));
 });
}

// Source-typed Future<int> -> native Rust compiler -> fully AOT Wasm ->
// JS v5 host -> real Node Worker MessagePort external completion.
// Guest code is never interpreted or generated at runtime.
import assert from 'node:assert/strict';
import {execFileSync} from 'node:child_process';
import {mkdtemp,readFile,writeFile,access,rm} from 'node:fs/promises';
import {Worker,MessageChannel} from 'node:worker_threads';
import {tmpdir} from 'node:os';
import {join} from 'node:path';
import {createWorkerControlLoop} from '../runtime/browser/worker-control-loop.mjs';
import {createPortableTaskHostV5,verifyPortableTaskGuestV5}
  from '../runtime/browser/portable-task-host-v5.mjs';

const source=`pub susp fnc main(): int {
  val key: int=41;
  val first: Future<int>=pending_future_int(key);
  await take! first;
  val second: Future<int>=pending_future_int(42);
  return await take! second;
}`;
const temp=await mkdtemp(join(tmpdir(),'atto-typed-future-'));
try{
  const path=join(temp,'typed.atto'),out=join(temp,'typed.wasm');
  await writeFile(path,source);
  execFileSync(process.env.ATTOC||'target/release/attoc',
    ['--future-int-aot',path,'-o',out],{stdio:'pipe',timeout:30000});
  const wasm=await readFile(out);
  assert.equal(WebAssembly.validate(wasm),true,'native compiler produced valid AOT Wasm');
  const module=new WebAssembly.Module(wasm);
  assert.equal(verifyPortableTaskGuestV5(module),true,'typed output executes against stable v5 ABI');
  const marker=WebAssembly.Module.customSections(module,'atto.browser.source.future.v1');
  assert.equal(marker.length,1);
  assert.deepEqual([...new Uint8Array(marker[0])],[1]);
  assert.equal(WebAssembly.Module.customSections(module,'atto.browser.guest.v5').length,1);
  assert.equal(WebAssembly.Module.exports(module).some(x=>x.kind==='memory'||x.kind==='table'),false);

  const worker=new Worker(`
    const {parentPort}=require('node:worker_threads');
    const channels=new Map(),held=[];let released=false;
    const send=data=>{
      const port=channels.get(data.owner);
      if(!port)throw Error('E_PENDING_PORT');
      port.postMessage({...data.grant,value:data.key===41n?111n:42n});
    };
    parentPort.on('message',msg=>{
      if(msg.kind==='bind')channels.set(msg.owner,msg.port);
      if(msg.kind==='grant'){if(released)send(msg);else held.push(msg);}
      if(msg.kind==='release'){released=true;for(const data of held.splice(0))send(data);}
    });
  `,{eval:true});
  const channels=[];
  try{
    const loop=createWorkerControlLoop({maxOwners:16,maxSuspended:16,maxReady:64,maxControl:4});
    const host=createPortableTaskHostV5({module,loop});
    let initial=0,callbacks=0,releaseFirst;
    const initialPending=new Promise(resolve=>releaseFirst=resolve);
    const tasks=Array.from({length:16},(_,i)=>{
      const owner='typed-future-'+i,{port1,port2}=new MessageChannel();
      channels.push(port1);
      worker.postMessage({kind:'bind',owner,port:port2},[port2]);
      return host.spawn({owner,ownerId:i+1,epoch:1,port:port1,
        onPending:({key,grant})=>{
          callbacks++;
          if(key===41n&&++initial===16)releaseFirst();
          worker.postMessage({kind:'grant',owner,key,grant});
        }});
    });
    await initialPending;
    await new Promise(resolve=>setTimeout(resolve,30));
    assert.equal(host.stats.liveFrames,16,'all frames live and pending');
    assert.equal(host.stats.starts,16);
    assert.equal(host.stats.resumes,0,'no eager resume while pending');
    assert.equal(host.stats.completions,0);
    assert.equal(host.stats.turns,16,'no polling-spin DATA turns');
    assert.equal(loop.stats.suspended,16);
    worker.postMessage({kind:'release'});
    const outputs=await Promise.all(tasks.map(task=>task.result));
    assert.deepEqual(outputs,Array(16).fill(42n),
      'typed Future<int> result is supplied by external completion');
    assert.equal(host.stats.completions,32);
    assert.equal(host.stats.resumes,32);
    assert.equal(host.stats.drops,16);
    assert.equal(host.stats.liveFrames,0);
    assert.equal(host.stats.peak,16);
    assert.equal(callbacks,32);
    assert.equal(loop.stats.owners,0);
    assert.equal(host.close(),true);
    console.log('PASS typed Future<int>: 16 compiled Wasm frames, 32 authenticated Worker completions, 0 premature resumes');
  }finally{
    channels.forEach(port=>port.close());
    await worker.terminate();
  }

  // No silent reinterpretation in default/v5 source targets.
  assert.throws(()=>execFileSync(process.env.ATTOC||'target/release/attoc',
    [path,'-o',out],{stdio:'pipe',timeout:30000}));
  assert.throws(()=>execFileSync(process.env.ATTOC||'target/release/attoc',
    ['--portable-v5',path,'-o',out],{stdio:'pipe',timeout:30000}));
  const invalid=join(temp,'moved.atto');
  await writeFile(invalid,`pub susp fnc main():int {
    val f:Future<int>=pending_future_int(41);
    await take! f; return await f;
  }`);
  const deniedOut=join(temp,'invalid-moved.wasm');
  assert.throws(()=>execFileSync(process.env.ATTOC||'target/release/attoc',
    ['--future-int-aot',invalid,'-o',deniedOut],{stdio:'pipe',timeout:30000}),
    'source must reject a second await of the moved Future<int>');
  await assert.rejects(access(deniedOut),{code:'ENOENT'},
    'rejected Future ownership must never emit a guest artifact');
  console.log('PASS typed Future<int>: non-opt-in profiles and use-after-move fail closed');
}finally{await rm(temp,{recursive:true,force:true});}

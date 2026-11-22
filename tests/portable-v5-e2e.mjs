// Real native Rust --portable-v5 AOT guest. This is NOT a scheduler mock.
import assert from 'node:assert/strict';
import {execFileSync} from 'node:child_process';
import {mkdtemp,readFile,writeFile,rm} from 'node:fs/promises';
import {tmpdir} from 'node:os';
import {join} from 'node:path';
import {Worker,MessageChannel} from 'node:worker_threads';
import {createWorkerControlLoop} from '../runtime/browser/worker-control-loop.mjs';
import {createPortableTaskHostV5,verifyPortableTaskGuestV5}
  from '../runtime/browser/portable-task-host-v5.mjs';

const temp=await mkdtemp(join(tmpdir(),'atto-portable-v5-'));
try{
  const src=join(temp,'pending.atto'),out=join(temp,'pending.wasm');
  await writeFile(src,
    'pub susp fnc main(): int { val key: int=41; await key; return await 42; }');
  execFileSync(process.env.ATTOC||'target/release/attoc',
    ['--portable-v5',src,'-o',out],{timeout:30000,stdio:'pipe'});
  const bytes=await readFile(out);
  assert.equal(WebAssembly.validate(bytes),true);
  const module=new WebAssembly.Module(bytes);
  const marker=WebAssembly.Module.customSections(module,'atto.browser.guest.v5');
  assert.equal(marker.length,1);
  assert.deepEqual([...new Uint8Array(marker[0])],[5]);
  for(const v of [3,4])
    assert.equal(WebAssembly.Module.customSections(module,'atto.browser.guest.v'+v).length,0);
  assert.equal(WebAssembly.Module.exports(module).some(x=>x.kind==='memory'||x.kind==='table'),false);
  assert.deepEqual(WebAssembly.Module.imports(module),
    [{module:'atto',name:'cooperate',kind:'function'}]);
  const api=new WebAssembly.Instance(module,{atto:{
    cooperate(){throw Error('E_V5_JSPI_FORBIDDEN');}
  }}).exports;
  assert.equal(api.task_capacity_v5(),16);
  assert.equal(api.task_start_v5(0,1),-1);
  assert.equal(api.task_start_v5(1,0),-1);
  const handles=Array.from({length:16},(_,i)=>api.task_start_v5(i+1,1));
  assert.equal(new Set(handles).size,16);
  assert.ok(handles.every(x=>Number.isSafeInteger(x)&&x>0));
  assert.equal(api.task_start_v5(99,1),-2);
  for(let i=0;i<16;i++){
    const h=handles[i],owner=i+1;
    assert.equal(api.task_pending_v5(h,owner,1),41n);
    assert.equal(api.task_resume_v5(h,owner,1),-1,'no eager progress before future is ready');
    assert.equal(api.task_complete_v5(h,owner+100,1,41n,42n),-1);
    assert.equal(api.task_complete_v5(h,owner,2,41n,42n),-1);
    assert.equal(api.task_complete_v5(h,owner,1,42n,42n),-1,'wrong future key');
    assert.equal(api.task_complete_v5(h,owner,1,41n,99n),0);
    assert.equal(api.task_complete_v5(h,owner,1,41n,99n),-1,'one-shot completion');
    assert.throws(()=>api.task_pending_v5(h,owner,1),WebAssembly.RuntimeError);
    assert.equal(api.task_resume_v5(h,owner,1),1,'next await is now pending');
    assert.equal(api.task_pending_v5(h,owner,1),42n);
    assert.equal(api.task_resume_v5(h,owner,1),-1,'second future still pending');
  }
  for(let i=15;i>=0;i--){
    const h=handles[i],owner=i+1;
    assert.equal(api.task_complete_v5(h,owner,1,42n,BigInt(i+42)),0);
    assert.equal(api.task_resume_v5(h,owner,1),2);
    assert.equal(api.task_result_v5(h,owner,1),BigInt(i+42),
      'result must come from authorized external completion, not literal future key');
    assert.equal(api.task_resume_v5(h,owner,1),-1);
    assert.equal(api.task_complete_v5(h,owner,1,42n,12n),-1);
    assert.equal(api.task_drop_v5(h,owner,1),0);
    assert.equal(api.task_drop_v5(h,owner,1),-1);
    assert.equal(api.task_resume_v5(h,owner,1),-1);
  }
  console.log('PASS portable v5: 16 AOT guest frames await two real external completions, no eager resume');

  const stale=handles[0],renew=api.task_start_v5(1,1);
  assert.ok(renew>0&&renew!==stale);
  assert.equal(api.task_complete_v5(stale,1,1,41n,42n),-1);
  assert.equal(api.task_resume_v5(stale,1,1),-1);
  assert.equal(api.task_cancel_v5(renew,1,1),4);
  assert.equal(api.task_complete_v5(renew,1,1,41n,42n),-1);
  assert.throws(()=>api.task_pending_v5(renew,1,1),WebAssembly.RuntimeError);
  assert.equal(api.task_drop_v5(renew,1,1),0);
  assert.equal(api.task_start_v5(1,2)>0,true);
  console.log('PASS portable v5: cancellation, ownership checks and generation-reuse fencing');

  const unsupported=join(temp,'bad.atto');
  await writeFile(unsupported,
    'pub susp fnc main(): int { val x: int=41+1; return await x; }');
  assert.throws(()=>execFileSync(process.env.ATTOC||'target/release/attoc',
    ['--portable-v5',unsupported,'-o',out],{timeout:30000,stdio:'pipe'}));
  console.log('PASS portable v5: fail-closed compiler admission');

  // Actual compiler-generated guest + exact JS host + real Node Worker.
  // Worker receives only a dedicated port and one bearer per future. No
  // completion may run guest code in the port's event handler.
  assert.equal(verifyPortableTaskGuestV5(module),true);
  const service=new Worker(`
    const {parentPort}=require('node:worker_threads');
    const ports=new Map(),queue=[];let released=false;
    function deliver(item){
      const port=ports.get(item.owner);
      if(!port)throw Error('E_V5_PORT_NOT_BOUND');
      const value=item.key===41n?111n:42n;
      port.postMessage({...item.grant,value});
    }
    parentPort.on('message',data=>{
      if(data.kind==='bind'){ports.set(data.owner,data.port);return;}
      if(data.kind==='grant') {
        if(released)deliver(data);
        else queue.push(data);
      }
      if(data.kind==='release'){
        released=true;
        for(const item of queue.splice(0))deliver(item);
      }
    });
  `,{eval:true});
  const channels=[];
  try{
    const loop=createWorkerControlLoop({maxOwners:16,maxSuspended:16,
      maxReady:64,maxControl:4});
    const host=createPortableTaskHostV5({module,loop});
    let firstKeys=0,published=0,signalFirst;
    const firstPending=new Promise(r=>signalFirst=r);
    const runs=Array.from({length:16},(_,i)=>{
      const owner='external-'+i;
      const {port1,port2}=new MessageChannel();
      channels.push(port1);
      service.postMessage({kind:'bind',owner,port:port2},[port2]);
      return host.spawn({owner,ownerId:i+1,epoch:1,port:port1,
        onPending:({key,grant})=>{
          published++;
          if(key===41n&&++firstKeys===16)signalFirst();
          service.postMessage({kind:'grant',owner,key,grant});
        }});
    });
    await firstPending;
    // Give the Worker time to receive grants; it has intentionally not
    // released them yet. A self-polling/cooperative v4-style runtime would
    // have resumed already; v5 must be quiescent.
    await new Promise(r=>setTimeout(r,25));
    assert.equal(host.stats.liveFrames,16);
    assert.equal(host.stats.starts,16);
    assert.equal(host.stats.resumes,0,'16 futures are truly pending');
    assert.equal(host.stats.completions,0);
    assert.equal(host.stats.turns,16);
    assert.equal(loop.stats.suspended,16);
    service.postMessage({kind:'release'});
    const values=await Promise.all(runs.map(t=>t.result));
    assert.deepEqual(values,Array(16).fill(42n));
    assert.equal(host.stats.completions,32);
    assert.equal(host.stats.resumes,32);
    assert.equal(host.stats.drops,16);
    assert.equal(host.stats.liveFrames,0);
    assert.equal(host.stats.peak,16);
    assert.equal(published,32);
    assert.equal(loop.stats.owners,0);
    assert.equal(host.close(),true);
    console.log('PASS portable v5: 16 real compiled Wasm futures await two Worker MessagePort completions with bearer authentication');
  }finally{
    for(const port of channels)port.close();
    await service.terminate();
  }
}finally{
  await rm(temp,{recursive:true,force:true});
}

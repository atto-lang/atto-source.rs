// Real native Rust attoc --portable-v4 -> one AOT Wasm instance.
// This exercises actual Wasm-owned frame slots, not a JS scheduler model.
import assert from 'node:assert/strict';
import {execFileSync} from 'node:child_process';
import {mkdtemp,readFile,writeFile,rm} from 'node:fs/promises';
import {tmpdir} from 'node:os';
import {join} from 'node:path';
import {createWorkerControlLoop} from '../runtime/browser/worker-control-loop-v1.mjs';
import {createPortableTaskArenaHostV4,verifyPortableTaskGuestV4}
  from '../runtime/browser/portable-task-host-v4.mjs';

const temp=await mkdtemp(join(tmpdir(),'atto-portable-v4-'));
try {
  const src=join(temp,'arena.atto'),out=join(temp,'arena.wasm');
  await writeFile(src,
    'pub susp fnc main(): int { val answer: int=42; await 41; return await answer; }');
  execFileSync(process.env.ATTOC||'target/release/attoc',
    ['--portable-v4',src,'-o',out],{timeout:30000,stdio:'pipe'});
  const bytes=await readFile(out);
  assert.equal(WebAssembly.validate(bytes),true,'emitted v4 is valid AOT Wasm');
  const module=new WebAssembly.Module(bytes);
  const marker=WebAssembly.Module.customSections(module,'atto.browser.guest.v4');
  assert.equal(marker.length,1);
  assert.deepEqual([...new Uint8Array(marker[0])],[4]);
  assert.equal(WebAssembly.Module.customSections(module,'atto.browser.guest.v3').length,0,
    'v4 does not masquerade as v3');
  assert.deepEqual(WebAssembly.Module.imports(module),
    [{module:'atto',name:'cooperate',kind:'function'}]);
  const exported=WebAssembly.Module.exports(module);
  assert.equal(exported.some(x=>x.kind==='memory'||x.kind==='table'),false,
    'private task arena is never exported as writable memory');
  const e=new WebAssembly.Instance(module,{atto:{
    cooperate(){throw Error('E_V4_JSPI_FORBIDDEN');}
  }}).exports;
  assert.equal(e.task_capacity_v4(),16);
  assert.equal(e.task_start_v4(0,1),-1);
  assert.equal(e.task_start_v4(1,0),-1);
  assert.equal(e.task_resume_v4(0,1,1),-1,'no zero handle');
  assert.equal(e.task_resume_v4(-1,1,1),-1,'negative handle rejected');

  const owners=Array.from({length:16},(_,i)=>100+i);
  const handles=owners.map(owner=>e.task_start_v4(owner,1));
  assert.equal(new Set(handles).size,16);
  assert.ok(handles.every(h=>h>16&&h>0));
  assert.equal(e.task_start_v4(999,1),-2,'arena quota, no allocation');
  assert.equal(e.task_resume_v4(handles[0],999,1),-1,'wrong owner');
  assert.equal(e.task_resume_v4(handles[0],owners[0],2),-1,'wrong epoch');
  assert.throws(()=>e.task_result_v4(handles[0],owners[0],1),
    WebAssembly.RuntimeError,'result unavailable before completion');

  // All tasks are simultaneously live. Interleave separately generated
  // continuation segments across the same single Wasm memory instance.
  for(let i=15;i>=0;i--) {
    assert.equal(e.task_resume_v4(handles[i],owners[i],1),1,'first await');
  }
  for(let i=0;i<16;i++) {
    assert.equal(e.task_resume_v4(handles[i],owners[i],1),2,'second await');
    assert.equal(e.task_result_v4(handles[i],owners[i],1),42n);
    assert.equal(e.task_resume_v4(handles[i],owners[i],1),-1,'completed replay denied');
  }
  assert.equal(e.task_start_v4(999,1),-2,'completed frames occupy until drop');
  for(let i=0;i<16;i++) {
    assert.equal(e.task_drop_v4(handles[i],owners[i],1),0);
    assert.equal(e.task_drop_v4(handles[i],owners[i],1),-1,'double free denied');
    assert.equal(e.task_resume_v4(handles[i],owners[i],1),-1,'stale handle invalid');
    assert.throws(()=>e.task_result_v4(handles[i],owners[i],1),
      WebAssembly.RuntimeError,'freed results unavailable');
  }
  console.log('PASS portable v4: 16 genuinely concurrent task frames in one AOT Wasm instance');

  const stale=handles[0];
  const reused=e.task_start_v4(owners[0],1);
  assert.ok(reused>0&&reused!==stale,'slot generation never aliases retired handle');
  assert.equal(e.task_resume_v4(stale,owners[0],1),-1,'stale generation rejected');
  assert.equal(e.task_cancel_v4(stale,owners[0],1),-1);
  assert.equal(e.task_cancel_v4(reused,owners[0],1),4);
  assert.equal(e.task_cancel_v4(reused,owners[0],1),-1,'double cancel rejected');
  assert.equal(e.task_resume_v4(reused,owners[0],1),-1,'cancelled frame cannot resume');
  assert.throws(()=>e.task_result_v4(reused,owners[0],1),
    WebAssembly.RuntimeError);
  assert.equal(e.task_drop_v4(reused,owners[0],1),0);
  console.log('PASS portable v4: generation fencing and cancellation/drop lifecycle');

  // A used slot is reclaimed, not left poisoned. The generation advances on
  // every reuse and remains valid across many bounded AOT cycles.
  let prev=reused;
  for(let epoch=2;epoch<=256;epoch++) {
    const handle=e.task_start_v4(owners[0],epoch);
    assert.ok(handle>prev,'no generation reuse after cancellation');
    assert.equal(e.task_resume_v4(handle,owners[0],epoch),1);
    assert.equal(e.task_resume_v4(handle,owners[0],epoch),2);
    assert.equal(e.task_result_v4(handle,owners[0],epoch),42n);
    assert.equal(e.task_drop_v4(handle,owners[0],epoch),0);
    assert.equal(e.task_resume_v4(prev,owners[0],epoch-1),-1);
    prev=handle;
  }
  console.log('PASS portable v4: 255 repeated generation advances and slot reclamations');

  // Opt-in is not a silent compiler downgrade.
  const negative=join(temp,'unsupported.atto');
  await writeFile(negative,
    'pub susp fnc main(): int { val x: int=41+1; await 41; return await x; }');
  assert.throws(()=>execFileSync(process.env.ATTOC||'target/release/attoc',
    ['--portable-v4',negative,'-o',out],{timeout:30000,stdio:'pipe'}),
    /Command failed/);
  console.log('PASS portable v4: unsupported guest shapes fail closed');

  // The real JS runtime (byte-identical to atto-source.js PR #121) hosts
  // multiple compiler-generated frames in a SINGLE Wasm instance.
  assert.equal(verifyPortableTaskGuestV4(module),true);
  const scheduled=[],runtimeLoop=createWorkerControlLoop({
    maxOwners:16,maxReady:64,maxControl:4,postTurn:fn=>scheduled.push(fn)});
  const runtime=createPortableTaskArenaHostV4({module,loop:runtimeLoop});
  const tasks=Array.from({length:16},(_,i)=>runtime.spawn({
    owner:'guest-'+i,ownerId:i+1,epoch:1
  }));
  assert.equal(runtime.stats.turns,0,'no guest execution during admission');
  let quanta=0;
  while(scheduled.length&&quanta<96) {
    scheduled.shift()();quanta++;
  }
  assert.deepEqual(await Promise.all(tasks.map(x=>x.result)),
    Array(16).fill(42n));
  assert.equal(runtime.stats.peak,16,'16 physical frames coexisted');
  assert.equal(runtime.stats.turns,48);
  assert.equal(runtime.stats.resumes,32);
  assert.equal(runtime.stats.cleanups,16);
  assert.equal(runtime.stats.liveFrames,0);
  assert.equal(runtimeLoop.stats.owners,0);
  assert.equal(runtime.close(),true);
  console.log('PASS portable v4: real JS host runs 16 compiled frames without JSPI');

  // CONTROL-priority revocation and closure after scheduler shutdown.
  const controlTurns=[],controlLoop=createWorkerControlLoop({
    postTurn:fn=>controlTurns.push(fn)});
  const controller=createPortableTaskArenaHostV4({module,loop:controlLoop});
  const cancelled=controller.spawn({owner:'cancelled',ownerId:7,epoch:1});
  cancelled.result.catch(()=>{});
  controlTurns.shift()(); // start allocates a Wasm frame
  assert.equal(cancelled.cancel(),true);
  controlTurns.shift()(); // CONTROL preempts queued DATA
  await assert.rejects(cancelled.result,/E_V4_CANCELLED/);
  assert.equal(controller.stats.cleanups,1);
  assert.equal(controller.stats.resumes,0);
  while(controlTurns.length)controlTurns.shift()();
  const shutdown=controller.spawn({owner:'shutdown',ownerId:8,epoch:1});
  shutdown.result.catch(()=>{});
  controlTurns.shift()(); // start another frame
  controlLoop.close(); // simulates lost scheduled turn on shutdown
  assert.equal(controller.close(),true,'owner can reclaim after loop close');
  await assert.rejects(shutdown.result,/E_V4_HOST_CLOSED/);
  assert.equal(controller.stats.cleanups,2);
  assert.equal(controller.stats.liveFrames,0);
  console.log('PASS portable v4: compiled frame cancellation and post-shutdown destruction');
} finally {
  await rm(temp,{recursive:true,force:true});
}

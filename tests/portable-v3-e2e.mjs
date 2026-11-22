// Real native Rust attoc -> AOT Wasm -> Node continuation ABI v3.
// No JSPI, interpreter, code generator in the runtime, or guest Promise await.
import assert from 'node:assert/strict';
import {execFileSync} from 'node:child_process';
import {mkdtemp,readFile,writeFile,rm} from 'node:fs/promises';
import {tmpdir} from 'node:os';
import {join} from 'node:path';
import {createWorkerControlLoop} from '../runtime/browser/worker-control-loop-v1.mjs';
import {createPortableTaskHostV3,verifyPortableTaskGuestV3}
  from '../runtime/browser/portable-task-host-v3.mjs';

const temp=await mkdtemp(join(tmpdir(),'atto-portable-v3-'));
try {
  const src=join(temp,'source.atto'),out=join(temp,'guest.wasm');
  await writeFile(src,'pub susp fnc main(): int { await 41; return await 42; }');
  execFileSync(process.env.ATTOC||'target/release/attoc',[src,'-o',out],
    {stdio:'pipe',timeout:30000});
  const bytes=await readFile(out);
  assert.equal(WebAssembly.validate(bytes),true);
  const module=new WebAssembly.Module(bytes);
  assert.equal(verifyPortableTaskGuestV3(module),true);
  const imports=WebAssembly.Module.imports(module);
  assert.deepEqual(imports,[{module:'atto',name:'cooperate',kind:'function'}]);

  // The guest is not permitted to execute the old stackful main entrypoint.
  const instance=new WebAssembly.Instance(module,{atto:{
    cooperate(){throw Error('E_JSPI_V3_FORBIDDEN');}
  }});
  const e=instance.exports;
  assert.equal(e.task_resume_v3(7,1),-1,'cannot resume an idle frame');
  assert.equal(e.task_start_v3(0,1),-1,'zero owner denied');
  assert.equal(e.task_start_v3(7,0),-1,'zero epoch denied');
  assert.equal(e.task_start_v3(7,1),1);
  assert.equal(e.task_start_v3(7,2),-1,'second live task denied');
  assert.equal(e.task_resume_v3(8,1),-1,'cross-owner rejected');
  assert.equal(e.task_resume_v3(7,2),-1,'cross-epoch rejected');
  assert.throws(()=>e.task_result_v3(7,1),WebAssembly.RuntimeError);
  assert.equal(e.task_resume_v3(7,1),1,'first await returns PENDING');
  assert.equal(e.task_resume_v3(7,1),2,'second await completes');
  assert.equal(e.task_result_v3(7,1),42n);
  assert.equal(e.task_resume_v3(7,1),-1,'duplicate resume denied');
  assert.equal(e.task_cancel_v3(7,1),-1,'cannot cancel completed task');
  assert.equal(e.task_drop_v3(7,1),0);
  assert.equal(e.task_drop_v3(7,1),-1,'double drop denied');
  assert.equal(e.task_start_v3(7,1),-1,'generation cannot be replayed');
  assert.equal(e.task_start_v3(7,2),1,'larger epoch accepted after drop');
  assert.equal(e.task_cancel_v3(7,2),4);
  assert.equal(e.task_resume_v3(7,2),-1,'cancelled task never resumes');
  assert.throws(()=>e.task_result_v3(7,2),WebAssembly.RuntimeError);
  assert.equal(e.task_drop_v3(7,2),0);
  assert.equal(e.task_start_v3(7,2),-1);
  assert.equal(e.task_start_v3(7,3),1);
  assert.equal(e.task_drop_v3(7,3),0,'dropping pending frame permitted');
  console.log('PASS portable v3: real guest generation, replay, cancel and drop lifecycle');

  const scheduler=[];
  const loop=createWorkerControlLoop({postTurn:fn=>scheduler.push(fn)});
  const host=createPortableTaskHostV3({module,loop,owner:'native-guest',ownerId:7,epoch:1});
  const p=host.run();
  assert.equal(scheduler.length,1);
  assert.equal(host.stats.turns,0,'no guest executes during admission');
  const step=()=>{assert.ok(scheduler.length,'expected scheduler quantum');scheduler.shift()();};
  step();assert.equal(host.stats.turns,1); // START
  step();assert.equal(host.stats.turns,2); // first await -> PENDING
  step();assert.equal(host.stats.turns,3); // second await -> DONE
  assert.equal(await p,42n);
  assert.equal(host.stats.resumes,2);
  assert.equal(host.stats.cleanups,1);
  assert.equal(host.stats.frameLive,false);
  assert.equal(loop.stats.owners,0,'owner retired after physical drop');
  assert.equal(host.cancel(),false,'settled task cannot be cancelled');
  console.log('PASS portable v3: Node scheduler executes three distinct guest quanta');

  const pendingTurns=[];
  const cancellationLoop=createWorkerControlLoop({postTurn:fn=>pendingTurns.push(fn)});
  const cancelled=createPortableTaskHostV3({
    module,loop:cancellationLoop,owner:'cancel-guest',ownerId:7,epoch:1});
  const aborted=cancelled.run();
  aborted.catch(()=>{});
  pendingTurns.shift()(); // allocate guest frame, queue DATA resume
  assert.equal(cancelled.stats.frameLive,true);
  assert.equal(cancelled.cancel(),true);
  assert.equal(cancelled.cancel(),false,'duplicate cancellation denied');
  pendingTurns.shift()(); // CONTROL cancels and drops before DATA
  await assert.rejects(aborted,/E_V3_CANCELLED/);
  assert.equal(cancelled.stats.cleanups,1,'cancel must physically release frame');
  while(pendingTurns.length)pendingTurns.shift()();
  assert.equal(cancelled.stats.resumes,0,'queued DATA must not resume after cancellation');
  assert.equal(cancellationLoop.stats.owners,0);
  console.log('PASS portable v3: cancellation CONTROL takes priority and drops frame');

  const noStartTurns=[];
  const beforeStartLoop=createWorkerControlLoop({postTurn:fn=>noStartTurns.push(fn)});
  const beforeStart=createPortableTaskHostV3({
    module,loop:beforeStartLoop,owner:'before-start',ownerId:7,epoch:1});
  const rejected=beforeStart.run();
  rejected.catch(()=>{});
  assert.equal(beforeStart.cancel(),true);
  noStartTurns.shift()(); // CONTROL cancels before guest starts
  await assert.rejects(rejected,/E_V3_CANCELLED/);
  assert.equal(beforeStart.stats.turns,0);
  assert.equal(beforeStart.stats.cleanups,0);
  while(noStartTurns.length)noStartTurns.shift()();
  console.log('PASS portable v3: cancel-before-start does not create a Wasm frame');

  const shutdownTurns=[];
  const shutdownLoop=createWorkerControlLoop({postTurn:fn=>shutdownTurns.push(fn)});
  const shutdown=createPortableTaskHostV3({
    module,loop:shutdownLoop,owner:'shutdown-guest',ownerId:7,epoch:1});
  const abandoned=shutdown.run();abandoned.catch(()=>{});
  shutdownTurns.shift()(); // allocate the Wasm frame; queue next resume
  assert.equal(shutdown.stats.frameLive,true);
  shutdownLoop.close(); // scheduler no longer able to dispatch CONTROL or DATA
  assert.equal(shutdown.cancel(),true,'explicit shutdown must remain revocable');
  await assert.rejects(abandoned,/E_V3_SCHEDULER_CLOSED/);
  assert.equal(shutdown.stats.frameLive,false,'shutdown must physically drop frame');
  assert.equal(shutdown.stats.cleanups,1);
  assert.equal(shutdown.stats.resumes,0,'no resume on closed scheduler');
  console.log('PASS portable v3: closed scheduler still revokes Promise and drops frame');


  // Compiler-generated i64 locals must survive real suspension in Wasm.
  const spillSource=join(temp,'spill.atto'),spillOut=join(temp,'spill.wasm');
  await writeFile(spillSource,
    'pub susp fnc main(): int { val answer: int = 42; await 41; return await answer; }');
  execFileSync(process.env.ATTOC||'target/release/attoc',[spillSource,'-o',spillOut],
    {stdio:'pipe',timeout:30000});
  const spillBytes=await readFile(spillOut);
  assert.equal(WebAssembly.validate(spillBytes),true);
  const spillModule=new WebAssembly.Module(spillBytes);
  assert.equal(verifyPortableTaskGuestV3(spillModule),true);
  const spillGuest=new WebAssembly.Instance(spillModule,{atto:{
    cooperate(){throw Error('E_V3_JSPI_FORBIDDEN');}
  }}).exports;
  assert.equal(spillGuest.task_start_v3(10,1),1);
  assert.equal(spillGuest.task_resume_v3(10,1),1);
  assert.equal(spillGuest.task_resume_v3(10,1),2);
  assert.equal(spillGuest.task_result_v3(10,1),42n,'live int local survived await');
  assert.equal(spillGuest.task_drop_v3(10,1),0);
  assert.equal(spillGuest.task_start_v3(10,1),-1,'old generation is fenced');
  console.log('PASS portable v3: actual AOT live-local spill survives resumption');

  // The shared host scheduler can service multiple independent task instances.
  // Each guest still owns one fixed private frame; this is NOT a shared arena.
  const multiTurns=[];
  const multiLoop=createWorkerControlLoop({postTurn:fn=>multiTurns.push(fn)});
  const first=createPortableTaskHostV3({
    module,loop:multiLoop,owner:'task-one',ownerId:7,epoch:1});
  const second=createPortableTaskHostV3({
    module:spillModule,loop:multiLoop,owner:'task-two',ownerId:8,epoch:1});
  const pFirst=first.run(),pSecond=second.run();
  assert.equal(multiLoop.stats.owners,2,'both task owners admitted');
  let multiQuanta=0;
  while(multiTurns.length&&multiQuanta<12) {
    multiTurns.shift()();
    multiQuanta++;
  }
  assert.equal(await pFirst,42n);
  assert.equal(await pSecond,42n);
  assert.equal(first.stats.turns,3);
  assert.equal(second.stats.turns,3);
  assert.equal(first.stats.cleanups,1);
  assert.equal(second.stats.cleanups,1);
  assert.equal(multiLoop.stats.owners,0,'both independent frames retired');
  assert.equal(multiQuanta,6);
  console.log('PASS portable v3: two distinct AOT tasks interleave on one scheduler');

} finally {
  await rm(temp,{recursive:true,force:true});
}

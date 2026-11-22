// Actual Rust-compiled ActorFuture<int> -> full AOT Wasm -> actor-incarnation
// fenced JS runtime -> genuine Node actor Worker and per-task MessagePorts.
import assert from 'node:assert/strict';
import {execFileSync} from 'node:child_process';
import {mkdtemp,writeFile,readFile,rm} from 'node:fs/promises';
import {Worker,MessageChannel} from 'node:worker_threads';
import {tmpdir} from 'node:os';
import {join} from 'node:path';
import {createWorkerControlLoop} from '../runtime/browser/worker-control-loop.mjs';
import {createActorFutureHostV1,verifyActorFutureGuestV1}
  from '../runtime/browser/actor-future-host-v1.mjs';

const source=`pub susp fnc main():int {
  val first:ActorFuture<int>=actor_future_int(7,41);
  await first;
  val second:ActorFuture<int>=actor_future_int(7,42);
  return await second;
}`;
const temp=await mkdtemp(join(tmpdir(),'atto-actor-future-'));
try {
  const input=join(temp,'actor.atto'),out=join(temp,'actor.wasm');
  await writeFile(input,source);
  execFileSync(process.env.ATTOC||'target/release/attoc',
    ['--actor-future-int-aot',input,'-o',out],{stdio:'pipe',timeout:30000});
  const bytes=await readFile(out);
  assert.ok(WebAssembly.validate(bytes),'native compiler emits valid full-AOT Wasm');
  const module=new WebAssembly.Module(bytes);
  assert.ok(verifyActorFutureGuestV1(module));
  assert.equal(WebAssembly.Module.exports(module).some(x=>x.kind==='memory'),false);

  const actor=new Worker(`
    const {parentPort}=require('node:worker_threads');
    let control,released=false;const pending=[],ports=new Map();
    function respond(msg){
      if(msg.actorId!==7||msg.incarnation!==1||
         ![41n,42n].includes(msg.key)||
         msg.requestIndex!==(msg.key===41n?0:1))
        throw Error('E_BAD_ACTOR_FUTURE_REQUEST');
      const reply=ports.get(msg.owner);
      if(!reply)throw Error('E_NO_REPLY_PORT');
      reply.postMessage({...msg.grant,value:msg.key===41n?111n:42n});
      reply.postMessage({...msg.grant,value:777n}); // duplicate: must be rejected
    }
    parentPort.on('message',data=>{
      if(data.kind==='bind'){
        control=data.port;
        control.on('message',msg=>{
          if(msg.type!=='atto.actor.future.request.v1')
            throw Error('E_ACTOR_FUTURE_PROTOCOL');
          if(msg.port)ports.set(msg.owner,msg.port);
          if(released)respond(msg);else pending.push(msg);
        });
        parentPort.postMessage('ready');
      }
      if(data.kind==='release'){
        released=true;
        for(const msg of pending.splice(0))respond(msg);
      }
    });
  `,{eval:true});
  const {port1,port2}=new MessageChannel();
  try {
    const ready=new Promise(resolve=>actor.once('message',resolve));
    actor.postMessage({kind:'bind',port:port2},[port2]);
    assert.equal(await ready,'ready');
    const loop=createWorkerControlLoop({maxOwners:16,maxSuspended:16,
      maxReady:64,maxControl:4});
    const notices=[];
    const host=createActorFutureHostV1({module,loop,
      onSupervisorNotice:n=>notices.push(n)});
    assert.equal(host.registerActor({actorId:7,incarnation:1,controlPort:port1}),true);
    const tasks=Array.from({length:16},(_,i)=>host.spawn({
      owner:'actorfuture-'+i,ownerId:i+1,epoch:1
    }));
    for(let attempt=0;attempt<70&&host.stats.requests!==16;attempt++)
      await new Promise(resolve=>setTimeout(resolve,5));
    assert.equal(host.stats.requests,16,'first 16 actor requests admitted');
    await new Promise(resolve=>setTimeout(resolve,25));
    assert.equal(host.stats.liveFrames,16,'all 16 guest frames remain live');
    assert.equal(host.stats.resumes,0,'no eager resume before actor replies');
    assert.equal(host.stats.completions,0);
    assert.equal(host.stats.peak,16);
    actor.postMessage({kind:'release'});
    const values=await Promise.all(tasks.map(t=>t.result));
    assert.deepEqual(values,Array(16).fill(42n));
    assert.equal(host.stats.requests,32);
    assert.equal(host.stats.completions,32);
    assert.equal(host.stats.resumes,32);
    assert.equal(host.stats.drops,16);
    assert.equal(host.stats.liveFrames,0);
    assert.equal(loop.stats.owners,0);
    assert.equal(host.stopActor({actorId:7,incarnation:1,kind:'terminated',reason:'worker exited'}),true);
    assert.deepEqual(notices,[{
      actorId:7,incarnation:1,kind:'terminated',reason:'worker exited'
    }]);
    assert.throws(()=>host.registerActor({
      actorId:7,incarnation:1,controlPort:port1
    }),/E_ACTOR_FUTURE_STALE_INCARNATION/);
    assert.equal(host.close(),true);
    console.log('PASS ActorFuture<int>: 16 real compiled actor-bound futures, 32 one-shot Worker replies, no eager resume and generation fencing');
  }finally {
    port1.close();
    await actor.terminate();
  }
  for(const flag of [null,'--portable-v5','--future-int-aot']){
    assert.throws(()=>execFileSync(process.env.ATTOC||'target/release/attoc',
      [...(flag?[flag]:[]),input,'-o',out],{stdio:'pipe',timeout:30000}));
  }
  const moved=join(temp,'moved.atto');
  await writeFile(moved,`pub susp fnc main():int {
    val r:ActorFuture<int>=actor_future_int(7,41);
    await r;
    return await r;
  }`);
  assert.throws(()=>execFileSync(process.env.ATTOC||'target/release/attoc',
    ['--actor-future-int-aot',moved,'-o',out],{stdio:'pipe',timeout:30000}));
  console.log('PASS ActorFuture<int>: non-opt-in targets and double-await fail closed');
}finally {await rm(temp,{recursive:true,force:true});}

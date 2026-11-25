// Real Rust-Wasm compiler and per-actor browser Web Workers.
// This private harness exposes host-only lifecycle evidence; it is NOT
// language-level get_done() or a supervisor priority CONTROL queue.
import {ActorLifecycle} from './actor-lifecycle.mjs';

self.onmessage=async ({data})=>{
  const actors=[];
  const actorCompletions=[];
  let stdout='',instance;

  function finalize(actor,status,reason) {
    if(actor.life.complete(status,reason))actorCompletions.push(actor.life.completion);
  }
  function stop(actor,status,reason) {
    finalize(actor,status,reason);
    actor.worker.terminate();
  }
  function abortRemaining(reason) {
    for(const actor of actors){
      if(!actor.life.completion)stop(actor,'TERMINATED',reason);
      else actor.worker.terminate();
    }
  }
  const resultEnvelope=(more)=>({...more,actorCompletions:[...actorCompletions]});
  const failureStatus=(error)=>
    /TIMEOUT|E_ACTOR_WORKER|E_ACTOR_CLONE|E_ACTOR_POST/.test(error.message)
      ?'TERMINATED':'FAILED';
  const failedActor=(actor,error)=>stop(actor,failureStatus(error),error.message);
  async function ensureReady(actor){
    try{await actor.ready}
    catch(error){failedActor(actor,error);throw error}
  }

  // Install each one-shot reply listener BEFORE posting the request.
  // Deadlines bound both unresponsive and unexpectedly exited workers.
  function request(actor,payload,expected,timeoutCode) {
    return new Promise((resolve,reject)=>{
      let settled=false;
      const finish=(err,value)=>{
        if(settled)return;
        settled=true;
        clearTimeout(timer);
        actor.worker.onmessage=null;
        actor.worker.onerror=null;
        actor.worker.onmessageerror=null;
        if(err)reject(err);else resolve(value);
      };
      const timer=setTimeout(()=>finish(new Error(timeoutCode)),2000);
      actor.worker.onmessage=({data:msg})=>{
        if(msg?.kind===expected)finish(null,msg);
        else finish(new Error(typeof msg?.message==='string'?msg.message:'E_ACTOR_PROTOCOL'));
      };
      actor.worker.onerror=()=>finish(new Error('E_ACTOR_WORKER'));
      actor.worker.onmessageerror=()=>finish(new Error('E_ACTOR_CLONE'));
      try{actor.worker.postMessage(payload)}catch(e){finish(new Error('E_ACTOR_POST: '+e.message))}
    });
  }

  async function deliver(actor,message,fromMailbox) {
    try{
      await ensureReady(actor);
      actor.life.draining();
      const reply=await request(actor,{kind:'message',value:message},'reply','E_ACTOR_TIMEOUT');
      if(typeof reply.value!=='bigint'||BigInt.asIntN(64,reply.value)!==reply.value)
        throw new Error('E_ACTOR_RESULT');
      if(fromMailbox)actor.life.acknowledged();
      else actor.life.acknowledgedExternal();
      stdout+='actor['+actor.life.handle+'] receive('+String(message)+') -> '+String(reply.value)+'\n';
    }catch(e){
      failedActor(actor,e);
      throw e;
    }
  }
  try{
    const response=await fetch('/atto_compiler.wasm',{cache:'no-store',credentials:'omit'});
    if(!response.ok)throw new Error('E_COMPILER_FETCH');
    const bytes=await response.arrayBuffer();
    if(!WebAssembly.validate(bytes))throw new Error('invalid compiler wasm');
    const compiler=new WebAssembly.Module(bytes);
    if(WebAssembly.Module.imports(compiler).length)throw new Error('unexpected compiler imports');
    const api=new WebAssembly.Instance(compiler,{}).exports;
    const encoded=new TextEncoder().encode(data.source);
    if(encoded.length>api.input_capacity())throw new Error('source too big');
    new Uint8Array(api.memory.buffer,api.input_ptr(),encoded.length).set(encoded);
    if(api.compile(encoded.length)!==0){
      if(api.result_length()!==0)throw new Error('compiler emitted on rejection');
      const mem=new Uint8Array(api.memory.buffer);const start=api.error_ptr();let end=start;
      while(end<mem.length&&mem[end]&&end-start<256)end++;
      self.postMessage(resultEnvelope({ok:false,error:new TextDecoder().decode(mem.subarray(start,end))}));
      return;
    }
    const out=new Uint8Array(api.memory.buffer,api.result_ptr(),api.result_length()).slice();
    if(!WebAssembly.validate(out))throw new Error('compiler emitted invalid wasm');
    const guest=new WebAssembly.Module(out);
    const allowed=new Set(['print_i64','print_bool','print_str','print_newline','actor_spawn','actor_send','actor_close']);
    for(const imp of WebAssembly.Module.imports(guest)){
      if(imp.module!=='atto'||!allowed.has(imp.name))throw new Error('guest import not granted: '+imp.module+'.'+imp.name);
    }
    const atto={
      print_i64:v=>{stdout+=String(v)},
      print_bool:v=>{stdout+=v?'true':'false'},
      print_str:(p,n)=>{stdout+=new TextDecoder('utf-8',{fatal:true}).decode(new Uint8Array(instance.exports.memory.buffer,p,n))},
      print_newline:()=>{stdout+='\n'},
      actor_spawn:id=>{
        if(!Number.isInteger(id)||id<0||actors.length>=4)throw new Error('E_ACTOR_LIMIT');
        const handle=actors.length+1;
        const worker=new Worker('/actor-worker.mjs',{type:'module'});
        const actor={worker,life:new ActorLifecycle(handle,id,2),ready:null};
        // The ready handshake is created before init, even if the child
        // responds while the main Wasm function is still running.
        actor.ready=request(actor,{kind:'init',actorId:id,wasm:out,
        testFailInit:data.actorFailureStage==='init',
        testFailShutdown:data.actorFailureStage==='shutdown'},'ready','E_ACTOR_READY_TIMEOUT')
          .then(()=>actor.life.ready());
        actor.ready.catch(()=>{}); // no unhandled rejection for an unused spawn
        actors.push(actor);
        return handle;
      },
      actor_send:(handle,message)=>{
        if(!Number.isInteger(handle)||!actors[handle-1])
          throw new Error('E_ACTOR_HANDLE');
        return actors[handle-1].life.send(message);
      },
      actor_close:handle=>{
        if(!Number.isInteger(handle)||!actors[handle-1])
          throw new Error('E_ACTOR_HANDLE');
        return Number(actors[handle-1].life.close());
      },
    };
    instance=new WebAssembly.Instance(guest,{atto});
    let value;
    try{
      value=instance.exports.main();
    }catch(e){
      const code=instance.exports.panic_code.value;
      const kind=code===1?'integer overflow':code===2?'division by zero':'trap: '+e.message;
      abortRemaining('E_GUEST_MAIN_PANIC');
      self.postMessage(resultEnvelope({ok:false,error:stdout+'E_PANIC: '+kind}));
      return;
    }

    // Test-only forced-worker failure injection. Never claim a shutdown
    // acknowledgement for a worker forcibly terminated by the host.
    if(data.actorForceTerminate!==undefined){
      const handle=data.actorForceTerminate;
      if(!Number.isInteger(handle)||!actors[handle-1])throw new Error('E_ACTOR_TEST_CONTROL');
      stop(actors[handle-1],'TERMINATED','E_ACTOR_FORCED');
      throw new Error('E_ACTOR_FORCED');
    }

    // The private guest main is synchronous. Therefore this is a bounded
    // admission queue, not evidence of arbitrary concurrent producer safety.
    for(const actor of actors) {
      await ensureReady(actor);
      actor.life.draining();
      while(true){
        const message=actor.life.dequeue();
        if(message===null)break;
        await deliver(actor,message,true);
      }
    }

    // Legacy test-only injected messages, separate from language send().
    if(data.actorMessages!==undefined){
      if(!Array.isArray(data.actorMessages)||data.actorMessages.length>4)throw new Error('E_ACTOR_LIMIT');
      for(const req of data.actorMessages){
        if(!req||!Number.isInteger(req.handle)||typeof req.value!=='string'
          ||! /^-?(0|[1-9][0-9]*)$/.test(req.value)){
          throw new Error('E_MESSAGE_TYPE: only canonical signed i64 payloads are admitted');
        }
        const value=BigInt(req.value);
        if(BigInt.asIntN(64,value)!==value)throw new Error('E_MESSAGE_RANGE');
        const actor=actors[req.handle-1];
        if(!actor)throw new Error('E_ACTOR_HANDLE');
        if(actor.life.admissionClosed)throw new Error('E_ACTOR_CLOSED');
        await deliver(actor,value,false);
      }
    }

    // A child acknowledges shutdown only after all prior messages and then
    // closes itself; FINISHED/CLOSED is not recorded merely on host terminate.
    for(const actor of actors){
      await ensureReady(actor);
      let done;
      try{
        done=await request(actor,{kind:'shutdown'},'done','E_ACTOR_SHUTDOWN_TIMEOUT');
        if(done.actorId!==actor.life.actorId
          ||done.processed!==actor.life.delivered+actor.life.externalDelivered)
          throw new Error('E_ACTOR_COMPLETION_COUNT');
      }catch(error){failedActor(actor,error);throw error}
      finalize(actor,actor.life.admissionClosed?'CLOSED':'FINISHED','E_ACTOR_SHUTDOWN_ACK');
      actor.worker.terminate(); // idempotent final resource cleanup
    }
    self.postMessage(resultEnvelope({ok:true,value:String(value),stdout}));
  }catch(e){
    abortRemaining('E_RUN_ABORT');
    self.postMessage(resultEnvelope({ok:false,error:'E_HOST: '+e.message}));
  }
};

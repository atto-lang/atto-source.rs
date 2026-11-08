// This worker loads the *real* Rust-compiled compiler wasm; no JS evaluator.
// Actor task completion is provided by the explicit child shutdown protocol.
import { ActorSession, ACTOR_ADMISSION } from './actor-lifecycle.mjs';
import { validate_actor_guest_imports } from './actor-import-contract.mjs';
self.onmessage=async ({data})=>{
  const actors=[];
  try{
    const bytes=await (await fetch('/atto_compiler.wasm')).arrayBuffer();
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
      self.postMessage({ok:false,error:new TextDecoder().decode(mem.subarray(start,end))});
      return;
    }
    const out=new Uint8Array(api.memory.buffer,api.result_ptr(),api.result_length()).slice();
    if(!WebAssembly.validate(out))throw new Error('compiler emitted invalid wasm');
    const guest=new WebAssembly.Module(out);
    // atto.browser.guest.v1: output imports and the gated actor_spawn capability.
    validate_actor_guest_imports(guest);
    let stdout='',instance;
    const awaited_handles=new Set();
    const atto={
      print_i64:v=>{stdout+=String(v)},
      print_bool:v=>{stdout+=v?'true':'false'},
      print_str:(p,n)=>{stdout+=new TextDecoder('utf-8',{fatal:true}).decode(new Uint8Array(instance.exports.memory.buffer,p,n))},
      print_newline:()=>{stdout+='\n'},
      actor_spawn:id=>{
        if(!Number.isInteger(id)||id<0||actors.length>=4)throw new Error('E_ACTOR_LIMIT');
        const child=new Worker('/actor-worker.mjs',{type:'module'});
        const actor=new ActorSession(child,id,out);
        actors.push(actor);
        return actors.length;
      },
      actor_send:(handle,value)=>{
        if(!Number.isInteger(handle)||handle<1||handle>actors.length)return ACTOR_ADMISSION.INVALID;
        return actors[handle-1].admit(value);
      },
      // Guest call only records the terminal join. main() is synchronous and
      // must not execute source instructions after this instruction. The host
      // asynchronously drains the actor and resolves only on its child 'done'.
      actor_join:handle=>{
        if(!Number.isInteger(handle)||handle<1||handle>actors.length)
          throw new Error('E_ACTOR_HANDLE');
        if(awaited_handles.has(handle))throw new Error('E_ACTOR_DUP_JOIN');
        awaited_handles.add(handle);
      },
    };
    instance=new WebAssembly.Instance(guest,{atto});
    let value;
    try{value=instance.exports.main()}
    catch(e){
      const code=instance.exports.panic_code.value;
      const kind=code===1?'integer overflow':code===2?'division by zero':'trap: '+e.message;
      for(const actor of actors)actor.cancel();
      self.postMessage({ok:false,error:stdout+'E_PANIC: '+kind});return;
    }
    try{
      if(data.actorMessages!==undefined){
        if(!Array.isArray(data.actorMessages)||data.actorMessages.length>4)
          throw new Error('E_ACTOR_LIMIT');
        for(const req of data.actorMessages){
          if(!req||!Number.isInteger(req.handle)||typeof req.value!=='string'||! /^-?(0|[1-9][0-9]*)$/.test(req.value)){
            throw new Error('E_MESSAGE_TYPE: only canonical signed i64 payloads are admitted');
          }
          const big=BigInt(req.value);
          if(BigInt.asIntN(64,big)!==big)throw new Error('E_MESSAGE_RANGE');
          const actor=actors[req.handle-1];
          if(!actor)throw new Error('E_ACTOR_HANDLE');
          if(actor.admit(big)!==ACTOR_ADMISSION.OK)throw new Error('E_ACTOR_MAILBOX_FULL');
        }
      }
      // The structured scope always drains and explicitly closes each child.
      // This is a real asynchronous task: worker messages and shutdown must be
      // acknowledged before a successful browser result is posted.
      for(let i=0;i<actors.length;i++){
        const actor=actors[i];
        const outcome=await actor.drain_and_close((message,reply)=>{
          stdout+='actor['+(i+1)+'] receive('+String(message)+') -> '+String(reply)+'\n';
        });
        if(outcome.kind!=='completed'||outcome.processed===undefined)
          throw new Error('E_ACTOR_DONE_PROTOCOL');
      }
      for(const handle of awaited_handles){
        const outcome=await actors[handle-1].get_done();
        if(outcome.kind!=='completed')throw new Error('E_ACTOR_DONE_PROTOCOL');
      }
    }finally{
      // Cancel only unfinished actors. A success cannot be manufactured by
      // worker.terminate() or by merely draining an admission queue.
      for(const actor of actors)if(!actor.is_terminal())actor.cancel();
    }
    self.postMessage({ok:true,value:String(value),stdout});
  }catch(e){for(const actor of actors)if(!actor.is_terminal())actor.cancel();self.postMessage({ok:false,error:'E_HOST: '+e.message})}
};

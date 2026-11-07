// This worker loads the *real* Rust-compiled compiler wasm; no JS evaluator.
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
    const allowed=new Set(['print_i64','print_bool','print_str','print_newline','actor_spawn']);
    for(const imp of WebAssembly.Module.imports(guest)){
      if(imp.module!=='atto'||!allowed.has(imp.name))throw new Error('guest import not granted: '+imp.module+'.'+imp.name);
    }
    let stdout='',instance;
    const atto={
      print_i64:v=>{stdout+=String(v)},
      print_bool:v=>{stdout+=v?'true':'false'},
      print_str:(p,n)=>{stdout+=new TextDecoder('utf-8',{fatal:true}).decode(new Uint8Array(instance.exports.memory.buffer,p,n))},
      print_newline:()=>{stdout+='\n'},
      actor_spawn:id=>{
        if(!Number.isInteger(id)||id<0||actors.length>=4)throw new Error('E_ACTOR_LIMIT');
        const worker=new Worker('/actor-worker.mjs',{type:'module'});
        const actor={worker,id};
        // Install the ready listener *before* init: the child may announce
        // readiness before the parent guest's main() returns.
        actor.ready=new Promise((resolve,reject)=>{
          const timer=setTimeout(()=>reject(new Error('E_ACTOR_READY_TIMEOUT')),2000);
          worker.onmessage=({data:msg})=>{
            clearTimeout(timer);
            if(msg?.kind==='ready')resolve();
            else reject(new Error(msg?.message||'E_ACTOR_PROTOCOL'));
          };
          worker.onerror=()=>{clearTimeout(timer);reject(new Error('E_ACTOR_WORKER'))};
        });
        // A spawn whose host never sends must not create an unhandled rejection.
        actor.ready.catch(()=>{});
        actors.push(actor);
        worker.postMessage({kind:'init',actorId:id,wasm:out});
        return actors.length;
      },
    };
    instance=new WebAssembly.Instance(guest,{atto});
    let value;
    try{value=instance.exports.main()}
    catch(e){
      const code=instance.exports.panic_code.value;
      const kind=code===1?'integer overflow':code===2?'division by zero':'trap: '+e.message;
      for(const actor of actors)actor.worker.terminate();
      self.postMessage({ok:false,error:stdout+'E_PANIC: '+kind});return;
    }
    try{
      if(data.actorMessages!==undefined){
        if(!Array.isArray(data.actorMessages)||data.actorMessages.length>4)throw new Error('E_ACTOR_LIMIT');
        for(const req of data.actorMessages){
          if(!req||!Number.isInteger(req.handle)||typeof req.value!=='string'||! /^-?(0|[1-9][0-9]*)$/.test(req.value)){
            throw new Error('E_MESSAGE_TYPE: only canonical signed i64 payloads are admitted');
          }
          const big=BigInt(req.value);
          if(BigInt.asIntN(64,big)!==big)throw new Error('E_MESSAGE_RANGE');
          const actor=actors[req.handle-1];
          if(!actor)throw new Error('E_ACTOR_HANDLE');
          await actor.ready;
          const result=await new Promise((resolve,reject)=>{
            const timer=setTimeout(()=>reject(new Error('E_ACTOR_TIMEOUT')),2000);
            actor.worker.onmessage=({data:msg})=>{
              clearTimeout(timer);
              if(msg?.kind==='reply'&&typeof msg.value==='bigint')resolve(msg.value);
              else reject(new Error(msg?.message||'E_ACTOR_PROTOCOL'));
            };
            actor.worker.onerror=()=>{clearTimeout(timer);reject(new Error('E_ACTOR_WORKER'))};
            actor.worker.postMessage({kind:'message',value:big});
          });
          stdout+='actor['+req.handle+'] receive('+req.value+') -> '+String(result)+'\n';
        }
      }
    }finally{for(const actor of actors)actor.worker.terminate()}
    self.postMessage({ok:true,value:String(value),stdout});
  }catch(e){for(const actor of actors)actor.worker.terminate();self.postMessage({ok:false,error:'E_HOST: '+e.message})}
};

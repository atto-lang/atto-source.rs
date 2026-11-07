// A private actor execution domain has one Web Worker and one independent
// WebAssembly.Instance. Shutdown ACK is sent only after earlier messages
// have completed. Errors are reported as host evidence, not as canonical
// priority supervisor CONTROL messages or language-level get_done().
let receiver=null,guest=null,actorId=null,processed=0,terminal=false;
self.onmessage=({data})=>{
  if(terminal)return;
  try{
    if(data?.kind==='init'){
      if(receiver)throw new Error('E_ACTOR_DUP_INIT');
      if(!Number.isInteger(data.actorId)||data.actorId<0||data.actorId>1024)throw new Error('E_ACTOR_ID');
      if(!(data.wasm instanceof Uint8Array)||!WebAssembly.validate(data.wasm))throw new Error('E_ACTOR_WASM');
      const module=new WebAssembly.Module(data.wasm);
      const allow=new Set(['actor_spawn','actor_send','actor_close','print_i64','print_bool','print_str','print_newline']);
      for(const imp of WebAssembly.Module.imports(module)){
        if(imp.module!=='atto'||!allow.has(imp.name))throw new Error('E_ACTOR_IMPORT');
      }
      const atto={
        actor_spawn:()=>{throw new Error('E_ACTOR_NESTED_SPAWN')},
        actor_send:()=>{throw new Error('E_ACTOR_CAPABILITY')},
        actor_close:()=>{throw new Error('E_ACTOR_CAPABILITY')},
        print_i64:()=>{throw new Error('E_ACTOR_STDIO')},
        print_bool:()=>{throw new Error('E_ACTOR_STDIO')},
        print_str:()=>{throw new Error('E_ACTOR_STDIO')},
        print_newline:()=>{throw new Error('E_ACTOR_STDIO')},
      };
      guest=new WebAssembly.Instance(module,{atto});
      receiver=guest.exports['atto_actor_receive_'+data.actorId];
      if(typeof receiver!=='function')throw new Error('E_ACTOR_RECEIVE_EXPORT');
      actorId=data.actorId;
      self.postMessage({kind:'ready'});
      return;
    }
    if(data?.kind==='message'){
      if(!receiver)throw new Error('E_ACTOR_NOT_READY');
      if(typeof data.value!=='bigint'||BigInt.asIntN(64,data.value)!==data.value)
        throw new Error('E_MESSAGE_TYPE');
      const result=receiver(data.value);
      if(typeof result!=='bigint')throw new Error('E_ACTOR_RESULT');
      processed++;
      self.postMessage({kind:'reply',value:result});
      return;
    }
    if(data?.kind==='shutdown'){
      if(!receiver)throw new Error('E_ACTOR_NOT_READY');
      terminal=true;
      self.postMessage({kind:'done',actorId,processed});
      self.close();
      return;
    }
    throw new Error('E_ACTOR_PROTOCOL');
  }catch(error){
    terminal=true;
    const code=guest?.exports?.panic_code?.value||0;
    const message=code?'E_ACTOR_PANIC_'+code:String(error.message);
    self.postMessage({kind:'error',actorId,message,panicCode:code});
    self.close();
  }
};

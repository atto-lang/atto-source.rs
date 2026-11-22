import {createChannelIntGuestV1,verifyChannelIntGuestV1,CHANNEL_INT_V1_IMPORTS} from '/runtime/browser/channel_int_guest_v1.mjs';
import {createWorkerControlLoop} from '/runtime/browser/worker-control-loop-v1.mjs';
import {createPortableTaskHostV3} from '/runtime/browser/portable-task-host-v3.mjs';
import {createJspiControlBridgeV1} from '/runtime/browser/wasm-jspi-control-bridge-v1.mjs';
// This worker loads the *real* Rust-compiled compiler wasm; no JS evaluator.
// Java Double.toString (JDK 19+ shortest round-trip); see tests/java-double.mjs
function javaDouble(x){
  if(Number.isNaN(x))return 'NaN';
  if(x===Infinity)return 'Infinity';
  if(x===-Infinity)return '-Infinity';
  if(x===0)return Object.is(x,-0)?'-0.0':'0.0';
  const a=Math.abs(x);
  if(a>=1e-3&&a<1e7){const s=String(x);return s.includes('.')?s:s+'.0';}
  const [m,e]=x.toExponential().split('e');
  return (m.includes('.')?m:m+'.0')+'E'+e.replace('+','');
}
self.onmessage=async ({data})=>{
  let channelRuntime,cooperateBridge,controlLoop;
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
    const asyncSections=WebAssembly.Module.customSections(guest,'atto.async');
    const useAsync=asyncSections.length===1&&new Uint8Array(asyncSections[0]).length===1&&new Uint8Array(asyncSections[0])[0]===2;
    if(asyncSections.length&&!useAsync)throw new Error('invalid async marker');
    // Explicitly selected AOT state-machine path: ordinary Wasm functions,
    // no WebAssembly.Suspending or JSPI stack retention. Keep legacy main
    // inaccessible to this host profile.
    if(data.portableV3===true){
      const taskLoop=createWorkerControlLoop({
        maxOwners:1,maxReady:8,maxControl:4,maxSuspended:1
      });
      const host=createPortableTaskHostV3({
        module:guest,loop:taskLoop,owner:'portable-wasm',ownerId:1,epoch:1
      });
      const result=await host.run();
      self.postMessage({ok:true,value:String(result),stdout:'',
        portableV3:true,portableTurns:host.stats.turns,
        portableResumes:host.stats.resumes,portableCleanups:host.stats.cleanups});
      return;
    }
    if(useAsync&&(typeof WebAssembly.Suspending!=='function'||typeof WebAssembly.promising!=='function'))throw new Error('JSPI unsupported');
    const useChannel=WebAssembly.Module.customSections(guest,'atto.channel_int').length>0;
    if(useChannel&&!verifyChannelIntGuestV1(out,guest))throw new Error('invalid channel ABI marker');
    const allowed=new Set(['print_i64','print_bool','print_str','print_newline','print_f64',...(useAsync?['sleep_ms','barrier','task_start','task_await','cooperate']:[]),...(useChannel?CHANNEL_INT_V1_IMPORTS:[])]);
    for(const imp of WebAssembly.Module.imports(guest)){
      if(imp.module!=='atto'||!allowed.has(imp.name))throw new Error('guest import not granted: '+imp.module+'.'+imp.name);
    }
    let stdout='',instance;
    const atto={
      print_i64:v=>{stdout+=String(v)},
      print_bool:v=>{stdout+=v?'true':'false'},
      print_str:(p,n)=>{stdout+=new TextDecoder('utf-8',{fatal:true}).decode(new Uint8Array(instance.exports.memory.buffer,p,n))},
      print_newline:()=>{stdout+='\n'},
      print_f64:v=>{stdout+=javaDouble(v)},
    };
    const jobs=new Map();let nextJob=1,barrierWaiters=[],cooperativeYields=0;
    if(useAsync){
      // atto.async/2 cooperative await enters the bounded synchronous-turn
      // scheduler, which *only* resolves the JSPI Promise at a later turn.
      // The Wasm stack resumes in a JSPI microtask, never inside DATA/CONTROL
      // dispatch. This is deliberately not a portable task_resume ABI.
      if(WebAssembly.Module.imports(guest).some(imp=>imp.module==='atto'&&imp.name==='cooperate')){
        controlLoop=createWorkerControlLoop({maxOwners:1,maxSuspended:64,maxReady:64,maxControl:4});
        cooperateBridge=createJspiControlBridgeV1({
          loop:controlLoop,owner:'browser-guest',epoch:1,maxPending:64
        });
        atto.cooperate=cooperateBridge.makeCooperateImport(guest);
      }
      atto.sleep_ms=new WebAssembly.Suspending(async ms=>{
        if(ms<0n||ms>1000n)throw new Error('invalid sleep duration');
        await new Promise(resolve=>setTimeout(resolve,Number(ms)));
        return 0n;
      });
      atto.barrier=new WebAssembly.Suspending(async n=>{
        if(n!==2n)throw new Error('invalid barrier party count');
        return new Promise(resolve=>{
          barrierWaiters.push(resolve);
          if(barrierWaiters.length===2){
            const ready=barrierWaiters;barrierWaiters=[];
            queueMicrotask(()=>ready.forEach(f=>f(0n)));
          }
        });
      });
      atto.task_start=id=>{
        if(jobs.size>=64||id<0||id>1024)throw new Error('task quota/index violation');
        const handle=nextJob++;
        let ok,bad;
        const p=new Promise((res,rej)=>{ok=res;bad=rej;});
        p.catch(()=>{});
        jobs.set(handle,{p});
        queueMicrotask(async()=>{
          try{const result=await WebAssembly.promising(instance.exports.task_invoke)(id);ok(result)}
          catch(error){bad(error)}
        });
        return handle;
      };
      atto.task_await=new WebAssembly.Suspending(async handle=>{
        const job=jobs.get(handle);
        if(!job)throw new Error('bad/consumed task handle');
        jobs.delete(handle);
        return await job.p;
      });
    }
    if(useChannel){
      channelRuntime=createChannelIntGuestV1({getMemory:()=>instance?.exports?.memory});
      Object.assign(atto,channelRuntime.imports);
      atto.channel_int_await_read_v1=new WebAssembly.Suspending(channelRuntime.imports.channel_int_await_read_v1);
    }
    instance=new WebAssembly.Instance(guest,{atto});
    let value;
    try{value=useAsync?await WebAssembly.promising(instance.exports.main)():instance.exports.main()}
    catch(e){
      const code=instance.exports.panic_code.value;
      const kind=code===1?'integer overflow':code===2?'division by zero':code===3?'heap exhausted':'trap: '+e.message;
      self.postMessage({ok:false,error:stdout+'E_PANIC: '+kind});return;
    }
    cooperativeYields=cooperateBridge?.stats.resolved??0;
    self.postMessage({ok:true,value:String(value),stdout,cooperativeYields,
      schedulerDispatches:controlLoop?.stats.completed??0});
  }catch(e){self.postMessage({ok:false,error:'E_HOST: '+e.message})}
  finally {
    // A guest can return/throw before awaiting a registered read. Dispose
    // synchronously on every exit, including panic and host error, rather
    // than leaving unclaimed tickets alive until garbage collection.
    cooperateBridge?.retire();
    channelRuntime?.host.dispose();
  }
};

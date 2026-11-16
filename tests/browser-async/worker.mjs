import {CheckedWriteQueue} from '/checked-write-queue.mjs';

// Execute the actual bounded host queue in the browser Worker, independently
// from Atto guest lowering. This probe must never be advertised as an
// executable nb/do guest write.
async function checkedWriterProbe() {
  const delivered=[],supervisor=[];
  let release;
  const gate=new Promise(resolve=>{release=resolve});
  const q=new CheckedWriteQueue({slots:2,maxBytes:12,
    sink:async bytes=>{
      delivered.push(new TextDecoder().decode(bytes));
      await gate;
      return {ok:true,written:bytes.length};
    },
    supervisor:event=>supervisor.push(event)
  });
  const first=q.submitWrite({owner:'actor',generation:7,mode:'nb',payload:'hello'});
  // A macrotask turn lets the first adapter call become committed.
  await new Promise(resolve=>setTimeout(resolve,0));
  const second=q.submitWrite({owner:'actor',generation:7,mode:'do',
    payload:'drop',onOutcome:()=>{throw new Error('cancelled callback must not run')}});
  const excess=q.submitWrite({owner:'actor',generation:7,mode:'nb',payload:'extra'});
  q.stopOwner('actor',7);
  release();
  const completed=await first.future;
  await new Promise(resolve=>setTimeout(resolve,0));
  return {
    first:completed,
    quota:excess.error?.code,
    queuedAccepted:second.ok,
    notices:supervisor.map(e=>e.outcome.error?.code),
    delivered,stats:q.stats,
  };
}

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
  if(data?.probe==='checked_writer'){
    try{self.postMessage({ok:true,probe:await checkedWriterProbe()});}
    catch(e){self.postMessage({ok:false,error:'E_HOST: '+e.message});}
    return;
  }
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
    if(useAsync&&(typeof WebAssembly.Suspending!=='function'||typeof WebAssembly.promising!=='function'))throw new Error('JSPI unsupported');
    const allowed=new Set(['print_i64','print_bool','print_str','print_newline','print_f64',...(useAsync?['sleep_ms','barrier','task_start','task_await','cooperate']:[])]);
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
      // Never shortcut an immediate await; release the guest Wasm stack and
      // reschedule on a fresh Worker event-loop turn instead of a microtask.
      atto.cooperate=new WebAssembly.Suspending(async ()=>{
        await new Promise(resolve=>setTimeout(resolve,0));
        cooperativeYields++;
      });
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
    instance=new WebAssembly.Instance(guest,{atto});
    let value;
    try{value=useAsync?await WebAssembly.promising(instance.exports.main)():instance.exports.main()}
    catch(e){
      const code=instance.exports.panic_code.value;
      const kind=code===1?'integer overflow':code===2?'division by zero':code===3?'heap exhausted':'trap: '+e.message;
      self.postMessage({ok:false,error:stdout+'E_PANIC: '+kind});return;
    }
    self.postMessage({ok:true,value:String(value),stdout,cooperativeYields});
  }catch(e){self.postMessage({ok:false,error:'E_HOST: '+e.message})}
};

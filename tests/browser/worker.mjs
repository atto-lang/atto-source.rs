// This worker loads the *real* Rust-compiled compiler wasm; no JS evaluator.
import {createStateRuntime} from './state-runtime.mjs';
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
    // atto.browser.guest.v1: only these output capabilities may be imported
    const allowed=new Set(['print_i64','print_bool','print_str','print_newline','print_f64',
      'state_register','state_enter','state_leave']);
    for(const imp of WebAssembly.Module.imports(guest)){
      if(imp.module!=='atto'||!allowed.has(imp.name))throw new Error('guest import not granted: '+imp.module+'.'+imp.name);
    }
    let stdout='',instance;
    const state=createStateRuntime();
    const atto={
      print_i64:v=>{stdout+=String(v)},
      print_bool:v=>{stdout+=v?'true':'false'},
      print_str:(p,n)=>{stdout+=new TextDecoder('utf-8',{fatal:true}).decode(new Uint8Array(instance.exports.memory.buffer,p,n))},
      print_newline:()=>{stdout+='\n'},
      print_f64:v=>{stdout+=javaDouble(v)},
      ...state.imports,
    };
    instance=new WebAssembly.Instance(guest,{atto});
    let value;
    try{value=instance.exports.main()}
    catch(e){
      // The synchronous WebAssembly call has physically unwound. Release
      // abandoned state leases only here, never while guest code still runs.
      state.abort();
      const code=instance.exports.panic_code.value;
      const kind=code===1?'integer overflow':code===2?'division by zero':code===3?'heap exhausted':'trap: '+e.message;
      self.postMessage({ok:false,error:stdout+'E_PANIC: '+kind});return;
    }
    state.complete(); // rejects any scope that failed to release its lease
    self.postMessage({ok:true,value:String(value),stdout});
  }catch(e){self.postMessage({ok:false,error:'E_HOST: '+e.message})}
};

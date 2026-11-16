// Real Atto source -> Rust wasm32 compiler -> guest Wasm Channel<int> JSPI.
// Unlike the separate synthetic-module ABI probe, this is a compiled guest.
import {createChannelIntGuestV1, verifyChannelIntGuestV1,
  CHANNEL_INT_V1_ERRORS as E} from '/runtime/browser/channel_int_guest_v1.mjs';
const source=`pub susp fnc read_channel_guest_v1(Channel<int> c): Result<Option<int>,IoError> {
    return << c;
}
pub fnc main(): int { return 0; }`;
const pause=()=>new Promise(resolve=>setTimeout(resolve,0));
self.onmessage=async()=>{
  try {
    if(typeof WebAssembly.Suspending!=='function'||typeof WebAssembly.promising!=='function')
      throw Error('JSPI unavailable');
    const compiled=await (await fetch('/atto_compiler.wasm')).arrayBuffer();
    const compiler=new WebAssembly.Instance(new WebAssembly.Module(compiled),{}).exports;
    const encoded=new TextEncoder().encode(source);
    new Uint8Array(compiler.memory.buffer,compiler.input_ptr(),encoded.length).set(encoded);
    if(compiler.compile(encoded.length)!==0) {
      const bytes=new Uint8Array(compiler.memory.buffer);
      let p=compiler.error_ptr(),q=p;
      while(q<bytes.length&&bytes[q]&&q-p<512)q++;
      throw Error('source compiler rejected: '+new TextDecoder().decode(bytes.subarray(p,q)));
    }
    const wasm=new Uint8Array(compiler.memory.buffer,compiler.result_ptr(),compiler.result_length()).slice();
    let module;
    try { module=new WebAssembly.Module(wasm); }
    catch(error) { throw Error('compiler generated invalid Wasm: '+String(error?.message||error)); }
    if(!verifyChannelIntGuestV1(wasm,module))throw Error('missing versioned channel marker');
    let instance,cooperativeYields=0;
    const {imports,host}=createChannelIntGuestV1({getMemory:()=>instance.exports.memory});
    const atto={...imports,
      channel_int_await_read_v1:new WebAssembly.Suspending(imports.channel_int_await_read_v1),
      cooperate:new WebAssembly.Suspending(async()=>{await pause();cooperativeYields++;})
    };
    instance=new WebAssembly.Instance(module,{atto});
    if(typeof instance.exports.read_channel_guest_v1!=='function')throw Error('read guest export missing');
    const invoke=WebAssembly.promising(instance.exports.read_channel_guest_v1);
    const view=()=>new DataView(instance.exports.memory.buffer);
    function decode(ptr){
      const v=view();
      if(!Number.isSafeInteger(ptr)||ptr<0||ptr+16>v.byteLength||ptr%8)throw Error('bad ADT pointer');
      const tag=v.getInt32(ptr,true);
      if(tag===1)return {kind:'Err',code:v.getInt32(ptr+8,true)};
      if(tag!==0)throw Error('bad Result tag');
      const opt=v.getInt32(ptr+8,true);
      if(opt<0||opt+16>v.byteLength||opt%8)throw Error('bad Option pointer');
      const sub=v.getInt32(opt,true);
      if(sub===0)return {kind:'None'};
      if(sub===1)return {kind:'Some',value:v.getBigInt64(opt+8,true)};
      throw Error('bad Option tag');
    }
    const opened=host.open(0);
    if(!opened.ok)throw Error('open failed');
    const ch=opened.value;
    const waiting=invoke(ch);
    for(let i=0;i<30&&host.snapshot(ch).value.pending!==1;i++)await pause();
    if(host.snapshot(ch).value.pending!==1)throw Error('compiled guest did not register pending read');
    if(!host.send(ch,41n).ok)throw Error('send admission failed');
    const first=decode(await waiting);
    if(first.kind!=='Some'||first.value!==41n)throw Error('typed Some(41) corrupted');
    const eofPending=invoke(ch);
    for(let i=0;i<30&&host.snapshot(ch).value.pending!==1;i++)await pause();
    if(host.snapshot(ch).value.pending!==1)throw Error('EOF read not registered');
    if(!host.close(ch).ok)throw Error('close failed');
    const eof=decode(await eofPending);
    if(eof.kind!=='None')throw Error('EOF fabricated a numeric value');
    const stale=decode(await invoke(2147483647));
    if(stale.kind!=='Err'||stale.code!==E.InvalidHandle)throw Error('invalid handle was not Err');
    if(!host.release(ch).ok)throw Error('closed/drained channel not released');
    // An immediately-ready zero must be preserved as Some(0), not EOF.
    const ready=host.open(1);
    if(!ready.ok||!host.send(ready.value,0n).ok)throw Error('buffered zero admission');
    const zero=decode(await invoke(ready.value));
    if(zero.kind!=='Some'||zero.value!==0n)throw Error('Some(0) conflated with EOF');
    host.close(ready.value);host.release(ready.value);
    if(cooperativeYields!==3)throw Error('exact cooperative yield count '+cooperativeYields);
    self.postMessage({ok:true,sourceCompiled:true,pendingSome:41,eof:true,
      invalidHandle:'Err',zeroSome:true,cooperativeYields});
  }catch(e){self.postMessage({ok:false,error:String(e?.stack||e)})}
};

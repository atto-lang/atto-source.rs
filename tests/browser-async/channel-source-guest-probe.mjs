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

    // Independently compile the declaration-first read from Atto source.
    // Same checked guest ABI, no synthetic Wasm, and a separate Worker-local
    // channel adapter/handle namespace; one read must dequeue one value.
    const declSource=source.replace('return << c;', 'val item << c; return item;');
    if(declSource===source)throw Error('declaration-first source not constructed');
    const declEncoded=new TextEncoder().encode(declSource);
    new Uint8Array(compiler.memory.buffer,compiler.input_ptr(),declEncoded.length).set(declEncoded);
    if(compiler.compile(declEncoded.length)!==0)throw Error('declaration-first guest compilation rejected');
    const declWasm=new Uint8Array(compiler.memory.buffer,compiler.result_ptr(),compiler.result_length()).slice();
    const declModule=new WebAssembly.Module(declWasm);
    if(!verifyChannelIntGuestV1(declWasm,declModule))throw Error('declaration-first guest lacks checked marker');
    let declInstance,declYields=0;
    const {imports:declImports,host:declHost}=createChannelIntGuestV1({
      getMemory:()=>declInstance.exports.memory});
    declInstance=new WebAssembly.Instance(declModule,{atto:{
      ...declImports,
      channel_int_await_read_v1:new WebAssembly.Suspending(declImports.channel_int_await_read_v1),
      cooperate:new WebAssembly.Suspending(async()=>{await pause();declYields++;})
    }});
    const declChannel=declHost.open(1);
    if(!declChannel.ok||!declHost.send(declChannel.value,42n).ok)
      throw Error('declaration-first channel admission failed');
    const declPtr=await WebAssembly.promising(declInstance.exports.read_channel_guest_v1)(declChannel.value);
    const declView=new DataView(declInstance.exports.memory.buffer);
    if(!Number.isInteger(declPtr)||declPtr<0||declPtr%8||declPtr+16>declView.byteLength)
      throw Error('declaration-first malformed result pointer');
    if(declView.getInt32(declPtr,true)!==0)
      throw Error('declaration-first result was not Ok');
    const optionPtr=declView.getInt32(declPtr+8,true);
    if(optionPtr<0||optionPtr%8||optionPtr+16>declView.byteLength
        ||declView.getInt32(optionPtr,true)!==1
        ||declView.getBigInt64(optionPtr+8,true)!==42n)
      throw Error('declaration-first read did not return Some(42)');
    if(declYields!==1)throw Error('declaration-first read must cooperate once');
    declHost.close(declChannel.value);declHost.release(declChannel.value);

    // Compile both additional one-shot spellings as *real Atto sources*.
    // Each gets a fresh Wasm module, channel host and private namespace.
    // This is not a synthetic ABI Wasm probe or a JavaScript read emulation.
    const aliases=[
      ['return readch c;',43n],
      ['c >> val item; return item;',44n],
    ];
    const aliasResults=[];
    for(const [body,expected] of aliases){
      const sourceAlias=source.replace('return << c;',body);
      if(sourceAlias===source)throw Error('alias fixture not constructed');
      const encodedAlias=new TextEncoder().encode(sourceAlias);
      new Uint8Array(compiler.memory.buffer,compiler.input_ptr(),encodedAlias.length).set(encodedAlias);
      if(compiler.compile(encodedAlias.length)!==0)throw Error('alias compilation failed: '+body);
      const binaryAlias=new Uint8Array(compiler.memory.buffer,compiler.result_ptr(),compiler.result_length()).slice();
      const moduleAlias=new WebAssembly.Module(binaryAlias);
      if(!verifyChannelIntGuestV1(binaryAlias,moduleAlias))throw Error('missing alias channel ABI marker');
      let aliasInstance,aliasYields=0;
      const {imports:aliasImports,host:aliasHost}=createChannelIntGuestV1({
        getMemory:()=>aliasInstance.exports.memory});
      aliasInstance=new WebAssembly.Instance(moduleAlias,{atto:{
        ...aliasImports,
        channel_int_await_read_v1:new WebAssembly.Suspending(aliasImports.channel_int_await_read_v1),
        cooperate:new WebAssembly.Suspending(async()=>{await pause();aliasYields++;})
      }});
      const openedAlias=aliasHost.open(1);
      if(!openedAlias.ok||!aliasHost.send(openedAlias.value,expected).ok)
        throw Error('alias channel admission failed: '+body);
      const ptr=await WebAssembly.promising(aliasInstance.exports.read_channel_guest_v1)(openedAlias.value);
      const memory=new DataView(aliasInstance.exports.memory.buffer);
      if(!Number.isInteger(ptr)||ptr<0||ptr%8||ptr+16>memory.byteLength
          ||memory.getInt32(ptr,true)!==0)throw Error('alias result not Ok');
      const inner=memory.getInt32(ptr+8,true);
      if(inner<0||inner%8||inner+16>memory.byteLength
          ||memory.getInt32(inner,true)!==1
          ||memory.getBigInt64(inner+8,true)!==expected)
        throw Error('alias value not Some('+expected+'): '+body);
      if(aliasYields!==1)throw Error('alias did not cooperate once: '+body);
      aliasHost.close(openedAlias.value);aliasHost.release(openedAlias.value);
      aliasResults.push(Number(expected));
    }
    // Source-created, bounded Worker-owned channel: open -> try_send ->
    // checked read -> close -> release, all compiled from Atto source.
    // Not a borrowed host-preopened handle or a synthetic Wasm module.
    const lifecycleBody=`val opened = Channel<int>.bounded(1);
      match opened over
        on Ok(local) -> {
          val sent = local.try_send(45);
          val received = readch local;
          val closed = local.close();
          val released = local.release();
          return received;
        }
        on Err(problem) -> { return Err(problem); }
      end`;
    const lifecycleSource=source.replace('return << c;',lifecycleBody);
    if(lifecycleSource===source)throw Error('channel lifecycle fixture missing');
    const lifecycleEncoded=new TextEncoder().encode(lifecycleSource);
    new Uint8Array(compiler.memory.buffer,compiler.input_ptr(),lifecycleEncoded.length)
      .set(lifecycleEncoded);
    if(compiler.compile(lifecycleEncoded.length)!==0) {
      const mem=new Uint8Array(compiler.memory.buffer),start=compiler.error_ptr();
      let end=start;while(end<mem.length&&mem[end]&&end-start<512)end++;
      throw Error('source lifecycle compilation: '+
        new TextDecoder().decode(mem.subarray(start,end)));
    }
    const lifecycleBinary=new Uint8Array(compiler.memory.buffer,compiler.result_ptr(),
        compiler.result_length()).slice();
    const lifecycleModule=new WebAssembly.Module(lifecycleBinary);
    if(!verifyChannelIntGuestV1(lifecycleBinary,lifecycleModule))
      throw Error('source lifecycle guest omitted channel ABI marker');
    let lifecycleInstance,lifecycleYields=0;
    const {imports:lifecycleImports,host:lifecycleHost}=createChannelIntGuestV1({
      getMemory:()=>lifecycleInstance.exports.memory});
    lifecycleInstance=new WebAssembly.Instance(lifecycleModule,{atto:{
      ...lifecycleImports,
      channel_int_await_read_v1:new WebAssembly.Suspending(lifecycleImports.channel_int_await_read_v1),
      cooperate:new WebAssembly.Suspending(async()=>{await pause();lifecycleYields++;})
    }});
    const lifecyclePtr=await WebAssembly.promising(
      lifecycleInstance.exports.read_channel_guest_v1)(0);
    const lifecycleView=new DataView(lifecycleInstance.exports.memory.buffer);
    if(!Number.isInteger(lifecyclePtr)||lifecyclePtr<0||lifecyclePtr%8
        ||lifecyclePtr+16>lifecycleView.byteLength
        ||lifecycleView.getInt32(lifecyclePtr,true)!==0)
      throw Error('source lifecycle Result not Ok');
    const lifecycleOption=lifecycleView.getInt32(lifecyclePtr+8,true);
    if(lifecycleOption<0||lifecycleOption%8||lifecycleOption+16>lifecycleView.byteLength
        ||lifecycleView.getInt32(lifecycleOption,true)!==1
        ||lifecycleView.getBigInt64(lifecycleOption+8,true)!==45n)
      throw Error('source lifecycle guest did not receive Some(45)');
    if(lifecycleYields!==1)throw Error('source lifecycle missed cooperative suspension');
    if(lifecycleHost.snapshot(1).ok)throw Error('guest did not release its owned channel');

    self.postMessage({ok:true,sourceCompiled:true,pendingSome:41,eof:true,
      invalidHandle:'Err',zeroSome:true,cooperativeYields,
      declarationFirstSome:42,declarationFirstYields:declYields,
      explicitReadchSome:aliasResults[0],directionalSome:aliasResults[1],
      sourceLifecycleSome:45,sourceLifecycleYields:lifecycleYields,sourceLifecycleReleased:true});
  }catch(e){self.postMessage({ok:false,error:String(e?.stack||e)})}
};

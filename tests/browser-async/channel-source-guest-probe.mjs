// Browser E2E: the REAL Rust/Wasm compiler emits an Atto source-compiled guest.
// The channel is host-created and passed through one strictly typed entry;
// source-level Channel<int>.bounded and general readch ABI remain separately gated.
import {createChannelIntGuestV1,verifyChannelIntGuestV1,CHANNEL_INT_V1_ERRORS as E} from
  '/runtime/browser/channel_int_guest_v1.mjs';
import {ChannelIntHostV1} from '/runtime/browser/channel_int_v1.mjs';

const src = body => 'pub susp fnc channel_read_guest(Channel<int> c): Result<Option<int>, IoError> { '+body+
  ' } pub fnc main(): int { return 0; }';
const assert=(test,why)=>{if(!test)throw Error(why)};
self.onmessage=async()=>{
  try {
    assert(typeof WebAssembly.Suspending==='function'&&typeof WebAssembly.promising==='function',
      'JSPI is required for a real suspending guest');
    const compilerBytes=await (await fetch('/atto_compiler.wasm')).arrayBuffer();
    const compiler=new WebAssembly.Instance(new WebAssembly.Module(compilerBytes),{}).exports;
    function compile(text) {
      const bytes=new TextEncoder().encode(text);
      assert(bytes.length<=compiler.input_capacity(),'compiler source cap');
      new Uint8Array(compiler.memory.buffer,compiler.input_ptr(),bytes.length).set(bytes);
      const status=compiler.compile(bytes.length);
      if(status!==0){
        assert(compiler.result_length()===0,'rejected compiler output must be empty');
        const m=new Uint8Array(compiler.memory.buffer),at=compiler.error_ptr();let n=0;
        while(n<300&&m[at+n])n++;
        return {error:new TextDecoder().decode(m.subarray(at,at+n))};
      }
      return {bytes:new Uint8Array(compiler.memory.buffer,compiler.result_ptr(),
        compiler.result_length()).slice()};
    }
    function instantiate(wasm) {
      const module=new WebAssembly.Module(wasm);
      assert(verifyChannelIntGuestV1(wasm,module),'typed channel ABI marker not verified');
      const host=new ChannelIntHostV1(), count={yields:0,registrations:0,lastTicket:0};
      let guest;
      const adapter=createChannelIntGuestV1({getMemory:()=>guest?.exports.memory,host});
      const atto={
        channel_int_nb_read_v1(c,out){
          const status=adapter.imports.channel_int_nb_read_v1(c,out);
          if(status===E.Ok){
            count.registrations++;
            count.lastTicket=new DataView(guest.exports.memory.buffer).getInt32(out,true);
          }
          return status;
        },
        channel_int_await_read_v1:new WebAssembly.Suspending(adapter.imports.channel_int_await_read_v1),
        cooperate:new WebAssembly.Suspending(async()=>{
          await new Promise(done=>setTimeout(done,0));count.yields++;
        }),
      };
      guest=new WebAssembly.Instance(module,{atto});
      assert(typeof guest.exports.channel_read_guest==='function','missing typed read guest entry');
      const read=WebAssembly.promising(guest.exports.channel_read_guest);
      const decode=ptr=>{
        const view=new DataView(guest.exports.memory.buffer);
        assert(Number.isInteger(ptr)&&ptr>=0&&ptr+16<=view.byteLength,'result pointer bounds');
        const tag=view.getInt32(ptr,true);
        if(tag===1)return {kind:'Err',error:view.getInt32(ptr+8,true)};
        assert(tag===0,'invalid outer ADT tag '+tag);
        const inner=view.getInt32(ptr+8,true);
        assert(inner>=0&&inner+16<=view.byteLength,'inner pointer bounds');
        const optionTag=view.getInt32(inner,true);
        if(optionTag===0)return {kind:'None'};
        assert(optionTag===1,'invalid inner Option tag '+optionTag);
        return {kind:'Some',value:view.getBigInt64(inner+8,true).toString()};
      };
      return {host,count,read,decode};
    }
    let results=[];
    const a=compile(src('return << c;'));
    assert(a.bytes,'prefix read must compile: '+a.error);
    const g=instantiate(a.bytes);
    const id=g.host.open(2).value;
    assert(g.host.send(id,41n).ok,'queue 41');
    assert(g.host.send(id,0n).ok,'queue zero');
    let received=g.decode(await g.read(id));
    assert(received.kind==='Some'&&received.value==='41','source-compiled Some(41)');
    received=g.decode(await g.read(id));
    assert(received.kind==='Some'&&received.value==='0','zero must be Some(0)');
    assert(g.count.registrations===2&&g.count.yields===2,'two exact reads and cooperate turns');
    g.host.close(id);
    received=g.decode(await g.read(id));
    assert(received.kind==='None','closed and drained channel is EOF None');
    assert(g.host.release(id).ok,'closed, drained channel releases');
    results.push('fifo-zero-eof');

    const h=instantiate(a.bytes),pendingId=h.host.open(0).value;
    const pending=h.read(pendingId);
    assert(h.count.lastTicket>0,'pending read registered without parking');
    assert(h.host.send(pendingId,42n).ok,'rendezvous delivery');
    received=h.decode(await pending);
    assert(received.kind==='Some'&&received.value==='42','pending guest resumed after host send');
    assert(h.count.yields===1,'pending guest cooperated');
    results.push('pending');

    const cancel=instantiate(a.bytes),cancelId=cancel.host.open(0).value;
    const p=cancel.read(cancelId);
    assert(cancel.host.cancel(cancel.count.lastTicket).ok,'cancel pending owned ticket');
    received=cancel.decode(await p);
    assert(received.kind==='Err'&&received.error===E.Cancelled,'guest must preserve Cancelled');
    received=cancel.decode(await cancel.read(999999));
    assert(received.kind==='Err'&&received.error===E.InvalidHandle,'invalid handle is checked Err');
    results.push('cancel-error');

    const two=compile(src('val first = << c; return << c;'));
    assert(two.bytes,'two distinct reads should compile: '+two.error);
    const j=instantiate(two.bytes),twice=j.host.open(2).value;
    j.host.send(twice,8n);j.host.send(twice,43n);
    received=j.decode(await j.read(twice));
    assert(received.kind==='Some'&&received.value==='43','second source read must dequeue second FIFO item');
    assert(j.count.registrations===2&&j.count.yields===2,'each prefix source occurrence is one read');
    results.push('two-receives');

    const explicit=compile(src('return readch c;'));
    assert(explicit.bytes,'explicit readch must share checked guest lowering: '+explicit.error);
    const k=instantiate(explicit.bytes),ch=k.host.open(1).value;
    k.host.send(ch,-9223372036854775808n);
    received=k.decode(await k.read(ch));
    assert(received.kind==='Some'&&received.value==='-9223372036854775808',
      'preserve full signed i64 in explicit readch');
    results.push('readch-i64-min');

    for(const [body,error] of [
      ['return << 42;', 'E_TYPE'],
      ['val int x = << c; return << c;', 'E_TYPE'],
    ]){
      const bad=compile(src(body));
      assert(!bad.bytes&&bad.error.includes(error),'must reject mis-typed read '+error);
    }
    const unsusp=compile('pub fnc channel_read_guest(Channel<int> c): Result<Option<int>,IoError> { return << c; } pub fnc main(): int { return 0; }');
    assert(!unsusp.bytes&&unsusp.error.includes('E_SUSPENDS_REQUIRED'),'unsuspended guest must reject');
    self.postMessage({ok:true,results,registered:g.count.registrations,yields:g.count.yields});
  }catch(error){self.postMessage({ok:false,error:String(error?.stack||error)});}
};
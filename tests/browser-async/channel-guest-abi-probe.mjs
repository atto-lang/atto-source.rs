// Real Chromium module Worker probing the versioned Channel<int> Wasm host
// adapter. This uses a hand-built Wasm import fixture, NOT AttoLang source
// compilation; the Rust compiler's guest read type checker remains gated.
import {createChannelIntGuestV1,verifyChannelIntGuestV1,
  CHANNEL_INT_V1_ERRORS as E,CHANNEL_INT_V1_TAGS as T} from '/runtime/browser/channel_int_guest_v1.mjs';

const leb=n=>{const a=[];do{let v=n&127;n=Math.floor(n/128);if(n)v|=128;a.push(v)}while(n);return a};
const name=s=>{const a=[...new TextEncoder().encode(s)];return [...leb(a.length),...a]};
const sec=(id,d)=>[id,...leb(d.length),...d];
const signature=(p,r)=>[0x60,p.length,...p,r.length,...r];
function guestWasm() {
  const body=[0,0x41,1,0x41,0,0x10,0,0x0b];
  return Uint8Array.from([
    0,97,115,109,1,0,0,0,
    ...sec(0,[...name('atto.channel_int'),1]),
    ...sec(0,[...name('atto.async'),2]),
    ...sec(1,[2,...signature([0x7f,0x7f],[0x7f]),...signature([],[0x7f])]),
    ...sec(2,[1,...name('atto'),...name('channel_int_open_v1'),0,0]),
    ...sec(3,[1,1]),
    ...sec(7,[1,...name('main'),0,1]),
    ...sec(10,[1,...leb(body.length),...body]),
  ]);
}
self.onmessage=async()=>{
  try {
    if(typeof WebAssembly.Suspending!=='function')throw Error('JSPI not available');
    const bytes=guestWasm(),module=new WebAssembly.Module(bytes);
    if(!verifyChannelIntGuestV1(bytes,module))throw Error('channel marker not admitted');
    const memory=new WebAssembly.Memory({initial:1});
    const {imports}=createChannelIntGuestV1({getMemory:()=>memory});
    const instance=new WebAssembly.Instance(module,{atto:imports});
    if(instance.exports.main()!==E.Ok)throw Error('guest import failed to create channel');
    const view=new DataView(memory.buffer),ch=view.getInt32(0,true);
    if(ch!==1)throw Error('bad channel handle');
    if(imports.channel_int_nb_read_v1(ch,4)!==E.Ok)throw Error('read registration');
    const ticket=view.getInt32(4,true),pending=imports.channel_int_await_read_v1(ticket,32);
    if(imports.channel_int_send_v1(ch,41n)!==E.Ok)throw Error('send admission');
    if(await pending!==E.Ok)throw Error('pending await');
    if(view.getInt32(32,true)!==T.Some||view.getInt32(36,true)!==E.Ok||
       view.getBigInt64(40,true)!==41n)throw Error('Some(i64) incorrect');
    if(await imports.channel_int_await_read_v1(ticket,48)!==E.Ok ||
       view.getInt32(48,true)!==T.Error||
       view.getInt32(52,true)!==E.AlreadyConsumed)
      throw Error('Future ticket replay accepted');
    if(imports.channel_int_nb_read_v1(ch,4)!==E.Ok)throw Error('EOF registration');
    const eofTicket=view.getInt32(4,true);
    if(imports.channel_int_close_v1(ch)!==E.Ok)throw Error('close');
    if(await imports.channel_int_await_read_v1(eofTicket,32)!==E.Ok||
       view.getInt32(32,true)!==T.None)
      throw Error('EOF lost or converted to sentinel');
    if(imports.channel_int_release_v1(ch)!==E.Ok)throw Error('release');
    self.postMessage({ok:true,importExecuted:true,some:41,eof:true,replay:'AlreadyConsumed',released:true});
  }catch(error){self.postMessage({ok:false,error:String(error)})}
};

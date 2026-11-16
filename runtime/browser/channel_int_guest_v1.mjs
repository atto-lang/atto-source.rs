// Checked, Worker-local Wasm import adapter for Channel<int> v1.
//
// Versioned compiler-emitted AttoLang guest imports are admitted only with
// the atto.channel_int v1 and atto.async v2 markers, exact Wasm function
// signatures, Worker-local ownership and checked tagged result slots.
//
// All operation failures are numeric statuses; await_read results use the
// checked 16-byte tagged payload:
//   i32 tag   @ +0  : 0 Ok(Some(i64)), 1 Ok(None), 2 Err(IoError)
//   i32 error @ +4  : 0 on success; checked IoError code on Err
//   i64 value @ +8  : signed 64-bit only for Ok(Some)
// No zero/sentinel conflation, host pointers, implicit unwrapping or JS
// exceptions for ordinary I/O errors.
import {ChannelIntHostV1} from './channel_int_v1.mjs';

export const CHANNEL_INT_ABI_MARKER = 'atto.channel_int';
export const CHANNEL_INT_ABI_VERSION = 1;
export const CHANNEL_INT_V1_ERRORS = Object.freeze({
  Ok: 0,
  InvalidCapacity: 1,
  Quota: 2,
  InvalidHandle: 3,
  InvalidValue: 4,
  Closed: 5,
  Full: 6,
  Cancelled: 7,
  AlreadyConsumed: 8,
  AlreadySettled: 9,
  NotClosed: 10,
  Busy: 11,
  InvalidPointer: 12,
});
export const CHANNEL_INT_V1_TAGS = Object.freeze({
  Some: 0,
  None: 1,
  Error: 2,
});
export const CHANNEL_INT_V1_IMPORTS = Object.freeze([
  'channel_int_open_v1',
  'channel_int_send_v1',
  'channel_int_nb_read_v1',
  'channel_int_await_read_v1',
  'channel_int_cancel_v1',
  'channel_int_close_v1',
  'channel_int_release_v1',
]);
const ERR = CHANNEL_INT_V1_ERRORS;
const TAG = CHANNEL_INT_V1_TAGS;
const types = Object.freeze({
  channel_int_open_v1: {params: [0x7f,0x7f], results:[0x7f]},
  channel_int_send_v1: {params: [0x7f,0x7e], results:[0x7f]},
  channel_int_nb_read_v1: {params: [0x7f,0x7f], results:[0x7f]},
  channel_int_await_read_v1: {params: [0x7f,0x7f], results:[0x7f]},
  channel_int_cancel_v1: {params: [0x7f], results:[0x7f]},
  channel_int_close_v1: {params: [0x7f], results:[0x7f]},
  channel_int_release_v1: {params: [0x7f], results:[0x7f]},
});
const identical=(a,b)=>a.length===b.length&&a.every((v,i)=>v===b[i]);
const status=answer=>answer.ok?ERR.Ok:(ERR[answer.error?.kind]??ERR.InvalidHandle);

// A read is an owned capability, not a transferable integer capability.
// The creating Worker constructs precisely one adapter per guest instance.
export function createChannelIntGuestV1({getMemory,host=new ChannelIntHostV1()}={}) {
  if (typeof getMemory!=='function') throw new TypeError('getMemory callback required');
  function view(out,len,alignment) {
    const memory=getMemory();
    if (!(memory instanceof WebAssembly.Memory)) return null;
    if (!Number.isInteger(out)||out<0||out%alignment!==0) return null;
    const buffer=memory.buffer;
    if (out>buffer.byteLength||len>buffer.byteLength-out) return null;
    return {memory,buffer,view:new DataView(buffer)};
  }
  function writeOutcome(out,answer,slot) {
    if (!slot||slot.memory!==getMemory()) return ERR.InvalidPointer;
    // Guest memory may have grown while suspended; revalidate immediately
    // before the write and never retain a DataView across await/JSPI.
    slot=view(out,16,8);
    if (!slot) return ERR.InvalidPointer;
    const {view:mem}=slot;
    const kind=answer.ok?(answer.value.some?TAG.Some:TAG.None):TAG.Error;
    const error=answer.ok?ERR.Ok:status(answer);
    const value=answer.ok&&answer.value.some?answer.value.value:0n;
    // All writes share one synchronous commit with no await between them.
    mem.setBigInt64(out+8,value,true);
    mem.setInt32(out+4,error,true);
    mem.setInt32(out,kind,true);
    return ERR.Ok;
  }
  const imports={
    channel_int_open_v1(capacity,out) {
      const slot=view(out,4,4);
      if (!slot) return ERR.InvalidPointer;
      const r=host.open(capacity);
      if (!r.ok) return status(r);
      // Validation and admission are synchronous; commit to that same
      // checked memory view, avoiding a second capability lookup.
      slot.view.setInt32(out,r.value,true);
      return ERR.Ok;
    },
    channel_int_send_v1(channel,value) {
      return status(host.send(channel,value));
    },
    channel_int_nb_read_v1(channel,out) {
      const slot=view(out,4,4);
      if (!slot) return ERR.InvalidPointer;
      const r=host.nb_read(channel);
      if (!r.ok) return status(r);
      // Validation and admission are synchronous; commit to that same
      // checked memory view, avoiding a second capability lookup.
      slot.view.setInt32(out,r.value,true);
      return ERR.Ok;
    },
    async channel_int_await_read_v1(ticket,out) {
      const slot=view(out,16,8);
      if (!slot) return ERR.InvalidPointer;
      const result=await host.await_read(ticket);
      return writeOutcome(out,result,slot);
    },
    channel_int_cancel_v1(ticket) {
      return status(host.cancel(ticket));
    },
    channel_int_close_v1(channel) {
      return status(host.close(channel));
    },
    channel_int_release_v1(channel) {
      return status(host.release(channel));
    },
  };
  return Object.freeze({imports:Object.freeze(imports),host});
}

// This is not a general Wasm decoder; it reads only types/imports and
// enforces the exact function signatures BEFORE guest instantiation.
// WebAssembly.Module.imports() alone does not report Wasm function types.
export function verifyChannelIntGuestV1(wasmBytes,wasmModule) {
  const bytes=wasmBytes instanceof Uint8Array?wasmBytes:new Uint8Array(wasmBytes);
  if (bytes.length<8||bytes.slice(0,8).some((x,i)=>x!==[0,97,115,109,1,0,0,0][i]))
    throw new Error('invalid Wasm header');
  const module=wasmModule??new WebAssembly.Module(bytes);
  const markers=WebAssembly.Module.customSections(module,CHANNEL_INT_ABI_MARKER);
  const imports=WebAssembly.Module.imports(module);
  const channelImports=imports.filter(x=>x.name.startsWith('channel_int_'));
  if (markers.length===0) {
    if (channelImports.length) throw new Error('unmarked channel import');
    return false;
  }
  if (markers.length!==1||new Uint8Array(markers[0]).length!==1||
      new Uint8Array(markers[0])[0]!==CHANNEL_INT_ABI_VERSION)
    throw new Error('unsupported/duplicate channel ABI marker');
  const asyncMarkers=WebAssembly.Module.customSections(module,'atto.async');
  if (asyncMarkers.length!==1||new Uint8Array(asyncMarkers[0]).length!==1||
      new Uint8Array(asyncMarkers[0])[0]!==2)
    throw new Error('channel imports require the version 2 async ABI');
  const admitted=new Set(CHANNEL_INT_V1_IMPORTS);
  const seen=new Set();
  for(const imp of channelImports) {
    if(imp.module!=='atto'||imp.kind!=='function'||!admitted.has(imp.name)||seen.has(imp.name))
      throw new Error('invalid/duplicate channel import: '+imp.name);
    seen.add(imp.name);
  }
  if(seen.size===0) throw new Error('marker without a channel import');
  let p=8,signatures=[],functionTypes=[];
  const readByte=()=>{if(p>=bytes.length)throw new Error('truncated Wasm');return bytes[p++]};
  const leb=()=>{let n=0,shift=0,b;
    do{b=readByte();if(shift>28)throw new Error('oversized Wasm LEB');
      n+=(b&127)*2**shift;shift+=7}while(b&128);
    if(n>0xffffffff)throw new Error('oversized Wasm index');
    return n;
  };
  const readName=()=>{const n=leb();if(n>bytes.length-p)throw new Error('truncated Wasm name');
    const value=new TextDecoder('utf-8',{fatal:true}).decode(bytes.subarray(p,p+n));p+=n;return value};
  const readVec=()=>{const n=leb();if(n>64)throw new Error('oversized Wasm signature');
    return Array.from({length:n},()=>readByte())};
  while(p<bytes.length) {
    const id=readByte(),size=leb(),end=p+size;
    if(end>bytes.length)throw new Error('truncated Wasm section');
    if(id===1) {
      const count=leb();if(count>10000)throw new Error('oversized type section');
      for(let i=0;i<count;i++) {
        if(readByte()!==0x60)throw new Error('unsupported function type');
        signatures.push({params:readVec(),results:readVec()});
      }
    } else if(id===2) {
      const count=leb();if(count>10000)throw new Error('oversized import section');
      for(let i=0;i<count;i++) {
        const namespace=readName(),name=readName(),kind=readByte();
        if(kind===0) {
          const typeIndex=leb();
          if(namespace==='atto'&&name.startsWith('channel_int_')) {
            const got=signatures[typeIndex],expected=types[name];
            if(!got||!expected||!identical(got.params,expected.params)||
               !identical(got.results,expected.results))
              throw new Error('wrong channel import signature: '+name);
            functionTypes.push(name);
          }
        } else if(kind===1) { // table descriptor and limits
          readByte();const flags=leb();leb();if(flags&1)leb();
        } else if(kind===2) { // memory descriptor
          const flags=leb();leb();if(flags&1)leb();
        } else if(kind===3) {readByte();readByte()}
        else throw new Error('unsupported import kind');
      }
    }
    if(p!==end && (id===1||id===2)) throw new Error('malformed Wasm type/import section');
    p=end;
  }
  if(functionTypes.length!==channelImports.length)
    throw new Error('Wasm imported signatures do not match module imports');
  return true;
}

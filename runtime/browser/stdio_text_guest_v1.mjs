// Checked, versioned, *trusted-host-injected* stdin/out/err guest Wasm ABI.
// No ambient process descriptors, shared worker handles or guest-chosen tenant
// identity. This module is a bridge; only a broker with verified tenant grants
// may call createStdioTextGuestV1.
import {StdioTextHostV1} from './stdio_text_host_v1.mjs';

export const STDIO_TEXT_ABI_MARKER = 'atto.stdio_text';
export const STDIO_TEXT_ABI_VERSION = 1;
export const STDIO_TEXT_ERRORS = Object.freeze({
  Ok:0, InvalidCapacity:1, Quota:2, InvalidHandle:3, InvalidValue:4,
  Closed:5, Full:6, Cancelled:7, AlreadyConsumed:8, AlreadySettled:9,
  NotClosed:10, Busy:11, InvalidPointer:12, Denied:13,
  MalformedUtf8:14, Revoked:15, RecordTooLarge:16, ReaderBusy:17,
  SourceFailure:18, SinkFailure:19, InvalidInput:20, InvalidOutcome:21,
});
export const STDIO_TEXT_V1_IMPORTS = Object.freeze([
  'stdio_text_read_v1','stdio_text_write_v1'
]);
const ERR=STDIO_TEXT_ERRORS;
const decoder=new TextDecoder('utf-8',{fatal:true});
const encoder=new TextEncoder();
const isI32=n=>Number.isSafeInteger(n)&&n>=0&&n<=0x7fffffff;
const knownStatus=answer=>answer.ok?ERR.Ok:(ERR[answer.error?.kind]??ERR.InvalidOutcome);

export function createStdioTextGuestV1({getMemory,host,grant}={}) {
  if(typeof getMemory!=='function')throw new TypeError('trusted guest memory accessor required');
  if(!(host instanceof StdioTextHostV1))throw new TypeError('trusted StdioTextHostV1 required');
  if(!grant || typeof grant!=='object' || Object.getPrototypeOf(grant)!==Object.prototype)
    throw new TypeError('broker-validated explicit stdio grant required');
  if(grant.tenantId!==host.tenantId || grant.generation!==host.generation ||
     !isI32(grant.generation) || !grant.generation ||
     !['allowStdin','allowStdout','allowStderr'].every(x=>typeof grant[x]==='boolean'))
    throw new TypeError('stdio tenant generation or permission grant mismatch');
  // Snapshot a trusted broker decision. Guest code sees only function imports.
  const {allowStdin,allowStdout,allowStderr}=grant;
  let revoked=false;
  function memoryRange(ptr,len,align) {
    if(!isI32(ptr)||!isI32(len)||ptr%align!==0)return null;
    const mem=getMemory();
    if(!(mem instanceof WebAssembly.Memory))return null;
    if(ptr>mem.buffer.byteLength||len>mem.buffer.byteLength-ptr)return null;
    return {memory:mem,buffer:mem.buffer,view:new DataView(mem.buffer)};
  }
  function commitRead(out,flat,bytes,dest,cap) {
    // Check after the JSPI suspension; memory views must never survive it.
    const destSlot=memoryRange(dest,cap,1);
    const outSlot=memoryRange(out,16,8);
    if(!outSlot||!destSlot||outSlot.memory!==destSlot.memory)
      return ERR.InvalidPointer;
    // Memory is revalidated after suspension. Never trust pre-await views.
    if(dest < out+16 && out < dest+cap)
      return ERR.InvalidPointer;
    if(flat.tag===0) {
      if(!bytes || bytes.length>cap)return ERR.InvalidOutcome;
      // One synchronous commit, no arbitrary guest callback between writes.
      new Uint8Array(destSlot.memory.buffer,dest,bytes.length).set(bytes);
    }
    outSlot.view.setInt32(out+12,0,true);
    outSlot.view.setInt32(out+8,flat.tag===0?bytes.length:0,true);
    outSlot.view.setInt32(out+4,flat.tag===2?flat.error:0,true);
    outSlot.view.setInt32(out,flat.tag,true);
    return ERR.Ok;
  }
  const imports={
    async stdio_text_read_v1(endpoint,dest,cap,out) {
      if(revoked)return ERR.Revoked;
      if(endpoint!==0 || !allowStdin)return ERR.Denied;
      if(!isI32(cap)||cap<1||cap>65536)return ERR.InvalidCapacity;
      if(!memoryRange(dest,cap,1)||!memoryRange(out,16,8))
        return ERR.InvalidPointer;
      // Reject overlapping result records and data *before* consuming any
      // stdin bytes. Without this, the post-JSPI flat tag/length stores can
      // overwrite part of an admitted line or permit a forged outcome.
      if(dest < out+16 && out < dest+cap)
        return ERR.InvalidPointer;
      let outcome;
      try {outcome=await host.stdio.in.readch()}
      catch {outcome={ok:false,error:{kind:'SourceFailure'}}}
      if(revoked)return ERR.Revoked;
      if(!outcome.ok)
        return commitRead(out,{tag:2,error:knownStatus(outcome)},null,dest,cap);
      if(!outcome.value.some)
        return commitRead(out,{tag:1},null,dest,cap);
      if(typeof outcome.value.value!=='string')
        return commitRead(out,{tag:2,error:ERR.InvalidOutcome},null,dest,cap);
      const bytes=encoder.encode(outcome.value.value);
      if(bytes.length>cap)
        return commitRead(out,{tag:2,error:ERR.RecordTooLarge},null,dest,cap);
      return commitRead(out,{tag:0},bytes,dest,cap);
    },
    async stdio_text_write_v1(endpoint,src,len) {
      if(revoked)return ERR.Revoked;
      if(endpoint===1?!allowStdout:endpoint===2?!allowStderr:true)return ERR.Denied;
      const range=memoryRange(src,len,1);
      if(!range)return ERR.InvalidPointer;
      if(len>65536)return ERR.Quota;
      // Copy before awaiting, so reentrant guest execution cannot mutate an
      // admitted output record while a scheduler turn is suspended.
      const bytes=new Uint8Array(range.buffer,src,len).slice();
      let value;
      try {value=decoder.decode(bytes)}catch{return ERR.MalformedUtf8}
      try {
        const answer=await (endpoint===1?host.stdio.out:host.stdio.err).writech(value);
        return knownStatus(answer);
      }catch{return ERR.SinkFailure}
    },
  };
  return Object.freeze({
    imports:Object.freeze(imports),
    revoke() {revoked=true;host.revoke()},
  });
}

// Validate marker, function imports AND Wasm function types before handing a
// compiler-produced module any tenant stdio capability. An arbitrary custom
// marker alone is never enough; incorrect signatures must fail admission.
export function verifyStdioTextGuestV1(wasmBytes,wasmModule) {
  const bytes=wasmBytes instanceof Uint8Array?wasmBytes:new Uint8Array(wasmBytes);
  if(bytes.length<8||bytes.slice(0,8).some((b,i)=>b!==[0,97,115,109,1,0,0,0][i]))
    throw Error('invalid Wasm header');
  const module=wasmModule??new WebAssembly.Module(bytes);
  const imports=WebAssembly.Module.imports(module);
  const stdioImports=imports.filter(x=>x.name.startsWith('stdio_text_'));
  const markers=WebAssembly.Module.customSections(module,STDIO_TEXT_ABI_MARKER);
  if(!markers.length) {
    if(stdioImports.length)throw Error('unmarked stdio import');
    return false;
  }
  if(markers.length!==1||new Uint8Array(markers[0]).length!==1||
     new Uint8Array(markers[0])[0]!==STDIO_TEXT_ABI_VERSION)
    throw Error('unsupported or duplicate stdio text ABI marker');
  const asyncMarkers=WebAssembly.Module.customSections(module,'atto.async');
  if(asyncMarkers.length!==1||new Uint8Array(asyncMarkers[0]).length!==1||
     new Uint8Array(asyncMarkers[0])[0]!==2)
    throw Error('stdio text I/O requires async JSPI ABI v2');
  const names=new Set(STDIO_TEXT_V1_IMPORTS),seen=new Set();
  for(const imp of stdioImports) {
    if(imp.module!=='atto'||imp.kind!=='function'||!names.has(imp.name)||seen.has(imp.name))
      throw Error('invalid/duplicate stdio import: '+imp.name);
    seen.add(imp.name);
  }
  if(!seen.size)throw Error('stdio marker without text imports');
  let p=8,signatures=[],actual=[];
  const byte=()=>{if(p>=bytes.length)throw Error('truncated Wasm');return bytes[p++]};
  const leb=()=>{let n=0,shift=0,b;
    do{b=byte();if(shift>28)throw Error('oversized LEB');n+=(b&127)*2**shift;shift+=7}while(b&128);
    if(n>0xffffffff)throw Error('oversized LEB value');return n;
  };
  const name=()=>{
    const size=leb();if(size>bytes.length-p)throw Error('truncated Wasm name');
    const result=decoder.decode(bytes.subarray(p,p+size));p+=size;return result;
  };
  const vec=()=>{
    const n=leb();if(n>64)throw Error('oversized function signature');
    return Array.from({length:n},()=>byte());
  };
  const same=(a,b)=>a.length===b.length&&a.every((value,i)=>value===b[i]);
  const expected={
    stdio_text_read_v1:{params:[0x7f,0x7f,0x7f,0x7f],results:[0x7f]},
    stdio_text_write_v1:{params:[0x7f,0x7f,0x7f],results:[0x7f]},
  };
  while(p<bytes.length) {
    const id=byte(),length=leb(),end=p+length;
    if(end>bytes.length)throw Error('truncated Wasm section');
    if(id===1) {
      const count=leb();if(count>10000)throw Error('oversized type section');
      for(let i=0;i<count;i++) {
        if(byte()!==0x60)throw Error('non-function type');
        signatures.push({params:vec(),results:vec()});
      }
    } else if(id===2) {
      const count=leb();if(count>10000)throw Error('oversized import section');
      for(let i=0;i<count;i++) {
        const ns=name(),symbol=name(),kind=byte();
        if(kind===0) {
          const index=leb();
          if(ns==='atto'&&symbol.startsWith('stdio_text_')) {
            const got=signatures[index],want=expected[symbol];
            if(!got||!want||!same(got.params,want.params)||!same(got.results,want.results))
              throw Error('wrong stdio import signature: '+symbol);
            actual.push(symbol);
          }
        } else if(kind===1) {byte();const f=leb();leb();if(f&1)leb()}
        else if(kind===2) {const f=leb();leb();if(f&1)leb()}
        else if(kind===3) {byte();byte()}
        else throw Error('unsupported import kind');
      }
    }
    if(p!==end && (id===1||id===2))throw Error('malformed Wasm type/import section');
    p=end;
  }
  if(actual.length!==seen.size)throw Error('stdio imported signatures mismatch');
  return true;
}

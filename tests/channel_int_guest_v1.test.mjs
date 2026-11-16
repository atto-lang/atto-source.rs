import test from 'node:test';
import assert from 'node:assert/strict';
import {
  createChannelIntGuestV1, verifyChannelIntGuestV1,
  CHANNEL_INT_V1_ERRORS as E, CHANNEL_INT_V1_TAGS as T,
} from '../runtime/browser/channel_int_guest_v1.mjs';

const fresh=()=>{const memory=new WebAssembly.Memory({initial:1});
  const {imports,host}=createChannelIntGuestV1({getMemory:()=>memory});
  const v=new DataView(memory.buffer);
  return {memory,imports,host,v};
};
const outcome=(v,p=32)=>({tag:v.getInt32(p,true),error:v.getInt32(p+4,true),value:v.getBigInt64(p+8,true)});
test('guest ABI: checked pending Future is a tagged Some(i64), never scalar sentinel',async()=>{
  const {imports:i,v}=fresh();
  assert.equal(i.channel_int_open_v1(0,0),E.Ok);
  const ch=v.getInt32(0,true);
  assert.equal(i.channel_int_nb_read_v1(ch,4),E.Ok);
  const ticket=v.getInt32(4,true);
  const pending=i.channel_int_await_read_v1(ticket,32);
  assert.equal(i.channel_int_send_v1(ch,0n),E.Ok);
  assert.equal(await pending,E.Ok);
  assert.deepEqual(outcome(v),{tag:T.Some,error:0,value:0n});
  assert.equal(await i.channel_int_await_read_v1(ticket,48),E.Ok);
  assert.deepEqual(outcome(v,48),{tag:T.Error,error:E.AlreadyConsumed,value:0n});
  assert.equal(i.channel_int_close_v1(ch),E.Ok);
  assert.equal(i.channel_int_release_v1(ch),E.Ok);
});
test('guest ABI: EOF is a distinct tag and never fabricates Some(0)',async()=>{
  const {imports:i,v}=fresh();
  assert.equal(i.channel_int_open_v1(1,0),E.Ok);
  const ch=v.getInt32(0,true);
  assert.equal(i.channel_int_send_v1(ch,41n),E.Ok);
  assert.equal(i.channel_int_close_v1(ch),E.Ok);
  assert.equal(i.channel_int_nb_read_v1(ch,4),E.Ok);
  assert.equal(await i.channel_int_await_read_v1(v.getInt32(4,true),32),E.Ok);
  assert.deepEqual(outcome(v),{tag:T.Some,error:0,value:41n});
  assert.equal(i.channel_int_nb_read_v1(ch,4),E.Ok);
  assert.equal(await i.channel_int_await_read_v1(v.getInt32(4,true),32),E.Ok);
  assert.deepEqual(outcome(v),{tag:T.None,error:0,value:0n});
  assert.equal(i.channel_int_release_v1(ch),E.Ok);
});
test('guest ABI: cancellation is Err(Cancelled), not EOF',async()=>{
  const {imports:i,v}=fresh();
  assert.equal(i.channel_int_open_v1(0,0),E.Ok);
  const ch=v.getInt32(0,true);
  assert.equal(i.channel_int_nb_read_v1(ch,4),E.Ok);
  const ticket=v.getInt32(4,true);
  const pending=i.channel_int_await_read_v1(ticket,32);
  assert.equal(i.channel_int_cancel_v1(ticket),E.Ok);
  assert.equal(await pending,E.Ok);
  assert.deepEqual(outcome(v),{tag:T.Error,error:E.Cancelled,value:0n});
  assert.equal(i.channel_int_close_v1(ch),E.Ok);
  assert.equal(i.channel_int_release_v1(ch),E.Ok);
});
test('guest ABI: closed channel, full buffer and stale handles produce checked statuses',()=>{
  const {imports:i,v}=fresh();
  assert.equal(i.channel_int_open_v1(1,0),E.Ok);
  const ch=v.getInt32(0,true);
  assert.equal(i.channel_int_send_v1(ch,-9223372036854775808n),E.Ok);
  assert.equal(i.channel_int_send_v1(ch,3n),E.Full);
  assert.equal(i.channel_int_close_v1(ch),E.Ok);
  assert.equal(i.channel_int_send_v1(ch,9n),E.Closed);
  assert.equal(i.channel_int_release_v1(ch),E.Busy);
  assert.equal(i.channel_int_send_v1(999,9n),E.InvalidHandle);
});
test('guest ABI: invalid pointer is rejected before registration or buffer mutation',()=>{
  const {imports:i,v,host}=fresh();
  v.setInt32(0,0x7f7f7f7f,true);
  assert.equal(i.channel_int_open_v1(1,65534),E.InvalidPointer);
  assert.equal(host.open(0).value,1,'invalid pointer must not leak quota');
  assert.equal(i.channel_int_nb_read_v1(1,65534),E.InvalidPointer);
  assert.equal(i.channel_int_nb_read_v1(1,-1),E.InvalidPointer);
  assert.equal(i.channel_int_nb_read_v1(1,3),E.InvalidPointer);
  assert.equal(v.getInt32(0,true),0x7f7f7f7f);
  assert.equal(host.snapshot(1).value.outstanding,0);
});
test('guest ABI: awaiting cannot write an unaligned/out-of-bounds result',async()=>{
  const {imports:i,v}=fresh();
  assert.equal(i.channel_int_open_v1(0,0),E.Ok);
  const ch=v.getInt32(0,true);
  assert.equal(i.channel_int_nb_read_v1(ch,4),E.Ok);
  const ticket=v.getInt32(4,true);
  assert.equal(await i.channel_int_await_read_v1(ticket,1),E.InvalidPointer);
  assert.equal(await i.channel_int_await_read_v1(ticket,65528),E.InvalidPointer);
  assert.equal(i.channel_int_cancel_v1(ticket),E.Ok,'invalid pointer cannot consume Future');
  assert.equal(await i.channel_int_await_read_v1(ticket,32),E.Ok);
  assert.deepEqual(outcome(v),{tag:T.Error,error:E.Cancelled,value:0n});
});
test('guest ABI: retained memory cannot be switched during pending suspend',async()=>{
  let memory=new WebAssembly.Memory({initial:1});
  const {imports:i}=createChannelIntGuestV1({getMemory:()=>memory});
  const view=new DataView(memory.buffer);
  assert.equal(i.channel_int_open_v1(0,0),E.Ok);
  assert.equal(i.channel_int_nb_read_v1(view.getInt32(0,true),4),E.Ok);
  const pending=i.channel_int_await_read_v1(view.getInt32(4,true),32);
  memory=new WebAssembly.Memory({initial:1});
  assert.equal(i.channel_int_cancel_v1(view.getInt32(4,true)),E.Ok);
  assert.equal(await pending,E.InvalidPointer);
});
test('guest ABI: unrelated modules remain unchanged, no marker means no permission',()=>{
  const minimal=Uint8Array.from([0,97,115,109,1,0,0,0]);
  assert.equal(verifyChannelIntGuestV1(minimal),false);
});

const uleb=n=>{const a=[];do{let b=n&127;n=Math.floor(n/128);if(n)b|=128;a.push(b)}while(n);return a};
const bytesName=s=>{const a=[...new TextEncoder().encode(s)];return [...uleb(a.length),...a]};
const section=(id,data)=>[id,...uleb(data.length),...data];
const func=(params,results)=>[0x60,...uleb(params.length),...params,...uleb(results.length),...results];
function tinyGuest({
  importName='channel_int_open_v1',namespace='atto',
  signature=[0x7f,0x7f],marker=1,asyncMarker=2,
  duplicateMarker=false,includeMain=true,duplicateImport=false,
}={}) {
  const markers=[
    ...(marker===null?[]:[section(0,[...bytesName('atto.channel_int'),marker])]),
    ...(duplicateMarker?[section(0,[...bytesName('atto.channel_int'),1])]:[]),
    ...(asyncMarker===null?[]:[section(0,[...bytesName('atto.async'),asyncMarker])]),
  ];
  const imports=[
    ...bytesName(namespace),...bytesName(importName),0,0,
    ...(duplicateImport?[...bytesName(namespace),...bytesName(importName),0,0]:[]),
  ];
  const typeSec=section(1,[includeMain?2:1,...func(signature,[0x7f]),...(includeMain?func([],[0x7f]):[])]);
  const importSec=section(2,[duplicateImport?2:1,...imports]);
  const base=[0,97,115,109,1,0,0,0,...markers.flat(),...typeSec,...importSec];
  if(!includeMain) return new Uint8Array(base);
  const body=[0,0x41,1,0x41,0,0x10,0,0x0b];
  return new Uint8Array([...base,...section(3,[1,1]),
    ...section(7,[1,...bytesName('main'),0,1]),
    ...section(10,[1,...uleb(body.length),...body])]);
}
test('guest ABI: signature verifier accepts genuine executable Wasm import',()=>{
  const bytes=tinyGuest();
  const module=new WebAssembly.Module(bytes);
  assert.equal(verifyChannelIntGuestV1(bytes,module),true);
  const {imports,memory}=fresh();
  const instance=new WebAssembly.Instance(module,{atto:imports});
  assert.equal(instance.exports.main(),E.Ok);
  assert.equal(new DataView(memory.buffer).getInt32(0,true),1);
});
test('guest ABI: unmarked/duplicated/unsupported version is rejected',()=>{
  for(const opts of [{marker:null},{marker:3},{duplicateMarker:true},{asyncMarker:null},{asyncMarker:1}]) {
    const bytes=tinyGuest({...opts,includeMain:false});
    assert.throws(()=>verifyChannelIntGuestV1(bytes),/marker|unmarked|version/i);
  }
});
test('guest ABI: reject forged import namespace/name/signature and duplicate slots',()=>{
  for(const opts of [
    {namespace:'forged'},
    {importName:'channel_int_fake_v1'},
    {signature:[0x7e,0x7f]},
    {duplicateImport:true},
  ]) {
    const bytes=tinyGuest({...opts,includeMain:false});
    assert.throws(()=>verifyChannelIntGuestV1(bytes),/import|signature|duplicate/i);
  }
});
test('guest ABI: close preserves buffered values until explicitly drained',async()=>{
  const {imports:i,v}=fresh();
  assert.equal(i.channel_int_open_v1(2,0),E.Ok);
  const ch=v.getInt32(0,true);
  assert.equal(i.channel_int_send_v1(ch,9223372036854775807n),E.Ok);
  assert.equal(i.channel_int_close_v1(ch),E.Ok);
  assert.equal(i.channel_int_release_v1(ch),E.Busy);
  assert.equal(i.channel_int_nb_read_v1(ch,4),E.Ok);
  assert.equal(await i.channel_int_await_read_v1(v.getInt32(4,true),32),E.Ok);
  assert.deepEqual(outcome(v),{tag:T.Some,error:0,value:9223372036854775807n});
  assert.equal(i.channel_int_release_v1(ch),E.Ok);
  assert.equal(i.channel_int_nb_read_v1(ch,4),E.InvalidHandle);
});

test('guest ABI: explicit abandon import rehomes a committed i64 without consuming it',async()=>{
  const {imports:i,v}=fresh();
  assert.equal(i.channel_int_open_v1(0,0),E.Ok);
  const ch=v.getInt32(0,true);
  assert.equal(i.channel_int_nb_read_v1(ch,4),E.Ok);
  const ticket=v.getInt32(4,true);
  assert.equal(i.channel_int_send_v1(ch,9223372036854775807n),E.Ok);
  assert.equal(i.channel_int_abandon_v1(ticket),E.Ok);
  assert.equal(i.channel_int_abandon_v1(ticket),E.AlreadyConsumed);
  assert.equal(i.channel_int_close_v1(ch),E.Ok);
  assert.equal(i.channel_int_release_v1(ch),E.Busy);
  assert.equal(i.channel_int_nb_read_v1(ch,4),E.Ok);
  assert.equal(await i.channel_int_await_read_v1(v.getInt32(4,true),32),E.Ok);
  assert.deepEqual(outcome(v),{tag:T.Some,error:0,value:9223372036854775807n});
  assert.equal(i.channel_int_release_v1(ch),E.Ok);
});

test('guest ABI: explicit cancel leaves Err(Cancelled) awaitable, abandon consumes it',async()=>{
  const {imports:i,v,host}=fresh();
  assert.equal(i.channel_int_open_v1(0,0),E.Ok);
  const ch=v.getInt32(0,true);
  assert.equal(i.channel_int_nb_read_v1(ch,4),E.Ok);
  const ticket=v.getInt32(4,true);
  assert.equal(i.channel_int_cancel_v1(ticket),E.Ok);
  assert.equal(i.channel_int_abandon_v1(ticket),E.Ok);
  assert.equal(host.snapshot(ch).value.outstanding,0);
  assert.equal(await i.channel_int_await_read_v1(ticket,32),E.Ok);
  assert.deepEqual(outcome(v),{tag:T.Error,error:E.AlreadyConsumed,value:0n});
  assert.equal(i.channel_int_close_v1(ch),E.Ok);
  assert.equal(i.channel_int_release_v1(ch),E.Ok);
});

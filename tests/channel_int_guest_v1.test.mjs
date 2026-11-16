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

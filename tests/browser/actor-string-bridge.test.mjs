import test from 'node:test';
import assert from 'node:assert/strict';
import {
  ActorStringMailBridge, ActorStringBridgeError,
  decodeStringDataFrame, unpackGuestString
} from './actor-string-bridge.mjs';

// Host boundary tests use WebAssembly.Memory objects, not compiled Atto guests.
// They prove copy/queue/fencing behavior only, NOT ActorMail<string> codegen.
const memory = pages => new WebAssembly.Memory({initial:pages, maximum:pages});
const value = (offset, length) => BigInt.asIntN(64, (BigInt(offset)<<32n) | BigInt(length));
const mk = (options = {}) => {
  const bridge = new ActorStringMailBridge(options);
  const from = memory(1), to = memory(2);
  const sender = bridge.register({tenant:'tenant-A',actorId:'sender',generation:1,
    memory:from,receives:null,sends:['string']});
  const receiver = bridge.register({tenant:'tenant-A',actorId:'receiver',generation:1,
    memory:to,receives:'string',sends:[],actorType:0,
    guestDataEnd:1200,reservation:{offset:65536,capacity:65536}});
  return {bridge,from,to,sender,receiver};
};
const fails = (fn, code) => assert.throws(fn,
  err => err instanceof ActorStringBridgeError && err.code === code, code);
const encode = (view, offset, s) => {
  const data = new TextEncoder().encode(s);
  new Uint8Array(view.buffer).set(data,offset);
  return value(offset,data.length);
};

test('owned UTF-8 data is copied across distinct Wasm memories then scrubbed', () => {
  const {bridge,from,to,sender,receiver}=mk();
  assert.notEqual(from.buffer,to.buffer);
  const ptr=encode(from,128,'hello 🦀');
  const result=bridge.sendPacked(sender,receiver,ptr);
  assert.deepEqual(result,{status:'ADMITTED',sequence:0});
  new Uint8Array(from.buffer).fill(0,128,148); // mutation after admission
  let handled='';
  const receipt=bridge.receiveNext(receiver,(type,payload,sequence)=>{
    assert.equal(type,0);
    assert.equal(sequence,0n);
    const {offset,length}=unpackGuestString(payload);
    handled=new TextDecoder('utf-8',{fatal:true}).decode(new Uint8Array(to.buffer,offset,length));
  });
  assert.equal(handled,'hello 🦀');
  assert.deepEqual(receipt,{status:'HANDLER_RETURNED',sequence:0});
  assert.ok(new Uint8Array(to.buffer,65536,16).every(b=>b===0));
});

test('strict UTF8, including malformed surrogate byte sequences, is denied',()=>{
  const {bridge,from,sender,receiver}=mk();
  const v=new Uint8Array(from.buffer);
  v.set([0xc0,0xaf],200);
  fails(()=>bridge.sendPacked(sender,receiver,value(200,2)),'E_ACTOR_UTF8');
  v.set([0xed,0xa0,0x80],200);
  fails(()=>bridge.sendPacked(sender,receiver,value(200,3)),'E_ACTOR_UTF8');
});

test('bounds, oversized byte count, wrong type and forged authority all fail before admission',()=>{
  const {bridge,from,sender,receiver}=mk();
  fails(()=>bridge.sendPacked({},receiver,value(0,0)),'E_ACTOR_CAPABILITY');
  fails(()=>bridge.sendPacked(sender,{},value(0,0)),'E_ACTOR_CAPABILITY');
  fails(()=>bridge.sendPacked(sender,receiver,value(65530,20)),'E_ACTOR_BOUNDS');
  fails(()=>bridge.sendPacked(sender,receiver,value(0,65537)),'E_ACTOR_QUOTA');
  fails(()=>bridge.sendPacked(sender,receiver,value(0,0),{kind:'CONTROL'}),'E_ACTOR_CONTROL_FORGED');
  const bad = bridge.register({tenant:'tenant-A',actorId:'int',generation:1,
    memory:memory(1),receives:'int',sends:[]});
  fails(()=>bridge.sendPacked(sender,bad,value(0,0)),'E_ACTOR_TYPE');
  const tenantB=bridge.register({tenant:'tenant-B',actorId:'b',generation:1,
    memory:memory(2),receives:'string',sends:[],guestDataEnd:4,
    reservation:{offset:65536,capacity:65536}});
  fails(()=>bridge.sendPacked(sender,tenantB,value(0,0)),'E_ACTOR_TENANT');
});

test('reservation rejects overlaps and out-of-memory locations',()=>{
  const bridge=new ActorStringMailBridge();
  fails(()=>bridge.register({tenant:'a',actorId:'b',generation:1,memory:memory(1),
    receives:'string',sends:[],guestDataEnd:1024,
    reservation:{offset:1000,capacity:16}}),'E_ACTOR_RESERVATION');
  fails(()=>bridge.register({tenant:'a',actorId:'b',generation:1,memory:memory(1),
    receives:'string',sends:[],guestDataEnd:1024,
    reservation:{offset:65530,capacity:16}}),'E_ACTOR_BOUNDS');
  fails(()=>bridge.register({tenant:'a',actorId:'b',generation:1,memory:memory(1),
    receives:'string',sends:[],guestDataEnd:1024}),'E_ACTOR_RESERVATION');
});

test('admission is bounded and differentiated from a handler completion',()=>{
  const {bridge,from,sender,receiver}=mk({maxMessages:1,maxQueuedBytes:20});
  const ptr=encode(from,128,'hello');
  assert.equal(bridge.sendPacked(sender,receiver,ptr).status,'ADMITTED');
  fails(()=>bridge.sendPacked(sender,receiver,ptr),'E_ACTOR_FULL');
  assert.equal(bridge.receiveNext(receiver,()=>{}).status,'HANDLER_RETURNED');
  assert.equal(bridge.receiveNext(receiver,()=>{}),null);
  assert.equal(bridge.sendPacked(sender,receiver,ptr).status,'ADMITTED');
  assert.equal(bridge.close(receiver),true);
  assert.equal(bridge.close(receiver),false);
  fails(()=>bridge.sendPacked(sender,receiver,ptr),'E_ACTOR_CLOSED');
});

test('closed actor handle is fenced across incarnation and stale references',()=>{
  const {bridge,from,sender,receiver}=mk();
  const ptr=encode(from,128,'x');
  bridge.sendPacked(sender,receiver,ptr);
  bridge.close(receiver);
  const next=bridge.register({tenant:'tenant-A',actorId:'receiver',generation:2,
    memory:memory(2),receives:'string',sends:[],guestDataEnd:4096,
    reservation:{offset:65536,capacity:65536},actorType:0});
  fails(()=>bridge.sendPacked(sender,receiver,ptr),'E_ACTOR_CLOSED');
  assert.equal(bridge.sendPacked(sender,next,ptr).status,'ADMITTED');
  assert.equal(bridge.receiveNext(next,()=>{}).status,'HANDLER_RETURNED');
});

test('a handler trap terminally closes the destination and never reports completion',()=>{
  const {bridge,from,sender,receiver}=mk();
  const ptr=encode(from,128,'x');
  bridge.sendPacked(sender,receiver,ptr);
  assert.throws(()=>bridge.receiveNext(receiver,()=>{throw new Error('guest trap');}),/guest trap/);
  fails(()=>bridge.receiveNext(receiver,()=>{}),'E_ACTOR_CLOSED');
  fails(()=>bridge.sendPacked(sender,receiver,ptr),'E_ACTOR_CLOSED');
});

test('strict ATAM frame denies forged kind, length, trailing, UTF8 and incomplete headers',()=>{
  const valid=Uint8Array.from([65,84,65,77,1,1,2,0,0,0,104,105]);
  assert.deepEqual(Array.from(decodeStringDataFrame(valid)),Array.from(valid));
  for(let n=0;n<10;n++)fails(()=>decodeStringDataFrame(valid.slice(0,n)),'E_ACTOR_TRUNCATED');
  for(const [i,v] of [[0,0],[4,2],[5,2]]){const bad=valid.slice();bad[i]=v;fails(()=>decodeStringDataFrame(bad),'E_ACTOR_FRAME');}
  const longer=valid.slice();longer[6]=3;fails(()=>decodeStringDataFrame(longer),'E_ACTOR_TRUNCATED');
  const shorter=valid.slice();shorter[6]=1;fails(()=>decodeStringDataFrame(shorter),'E_ACTOR_TRAILING');
  const invalid=valid.slice();invalid[10]=0xff;fails(()=>decodeStringDataFrame(invalid),'E_ACTOR_UTF8');
});

test('exact maximum length 65536 bytes can be delivered into reserved second page',()=>{
  const {bridge,from,sender,receiver}=mk({maxQueuedBytes:65546});
  new Uint8Array(from.buffer).fill(97);
  bridge.sendPacked(sender,receiver,value(0,65536));
  let len;
  bridge.receiveNext(receiver,(_type,packed)=>{len=unpackGuestString(packed).length;});
  assert.equal(len,65536);
});

test('one Wasm memory cannot be registered for multiple actor incarnations or tenants',()=>{
  const {bridge,from}=mk();
  fails(()=>bridge.register({tenant:'tenant-B',actorId:'evil',generation:1,
    memory:from,receives:'int',sends:[]}), 'E_ACTOR_MEMORY_ALIAS');
});

test('reentrant receive is refused and close during handler cannot return a success receipt',()=>{
  const {bridge,from,sender,receiver}=mk();
  const ptr=encode(from,128,'nested');
  bridge.sendPacked(sender,receiver,ptr);
  bridge.sendPacked(sender,receiver,ptr);
  fails(()=>bridge.receiveNext(receiver,()=>{
    fails(()=>bridge.receiveNext(receiver,()=>{}),'E_ACTOR_BUSY');
    bridge.close(receiver);
  }), 'E_ACTOR_CLOSED');
  fails(()=>bridge.receiveNext(receiver,()=>{}),'E_ACTOR_CLOSED');
});


// Tiny, independently assembled Wasm guest stub. This is NOT an Atto compiler
// artifact; it verifies the host passes the pointer/length to a real Wasm call.
function tinyWasmReceiver() {
  const section=(type,body)=>Uint8Array.from([type,body.length,...body]);
  const sig=section(1,[1,0x60,3,0x7f,0x7e,0x7e,0]); // (i32,i64,i64)->void
  const funcs=section(3,[1,0]);
  const mem=section(5,[1,1,2,2]); // two pages, fixed
  const exports=section(7,[
    2,
    6, ...Buffer.from('memory'), 2,0,
    13,...Buffer.from('actor_receive'),0,0,
  ]);
  // Store the first byte of the received guest string at memory[0].
  const body=[0,0x41,0,0x20,1,0x42,32,0x88,0xa7,0x2d,0,0,0x3a,0,0,0x0b];
  const code=section(10,[1,body.length,...body]);
  const bytes=Buffer.concat([Buffer.from([0,97,115,109,1,0,0,0]),sig,funcs,mem,exports,code]);
  assert.ok(WebAssembly.validate(bytes));
  return new WebAssembly.Instance(new WebAssembly.Module(bytes));
}

test('actual Wasm exported function reads owned string copied into isolated memory',()=>{
  const guest=tinyWasmReceiver();
  const bridge=new ActorStringMailBridge();
  const from=memory(1);
  const sender=bridge.register({tenant:'A',actorId:'s',generation:1,memory:from,
    receives:null,sends:['string']});
  const receiver=bridge.register({tenant:'A',actorId:'r',generation:1,
    memory:guest.exports.memory,receives:'string',sends:[],actorType:0,
    guestDataEnd:4096,reservation:{offset:65536,capacity:65536}});
  bridge.sendPacked(sender,receiver,encode(from,512,'hello Wasm'));
  assert.equal(bridge.receiveNext(receiver,guest.exports.actor_receive).status,'HANDLER_RETURNED');
  assert.equal(new Uint8Array(guest.exports.memory.buffer)[0],104); // 'h'
});


test('two independent destination memories receive identical fanout without aliasing',()=>{
  const {bridge,from,sender,receiver}=mk();
  const to2=memory(2);
  const second=bridge.register({tenant:'tenant-A',actorId:'receiver2',generation:1,
    memory:to2,receives:'string',sends:[],guestDataEnd:1000,
    reservation:{offset:65536,capacity:65536},actorType:1});
  const ptr=encode(from,512,'fanout');
  assert.equal(bridge.sendPacked(sender,receiver,ptr).sequence,0);
  assert.equal(bridge.sendPacked(sender,second,ptr).sequence,0);
  const first=bridge.receiveNext(receiver,()=>{});
  assert.equal(first.status,'HANDLER_RETURNED');
  assert.equal(bridge.receiveNext(second,(type,packed)=>{
    assert.equal(type,1);
    assert.equal(unpackGuestString(packed).length,6);
    assert.equal(new TextDecoder().decode(new Uint8Array(to2.buffer,65536,6)),'fanout');
  }).status,'HANDLER_RETURNED');
});

test('wire golden ATAM string hello is canonical and never CONTROL',()=>{
  const golden=Uint8Array.from([65,84,65,77,1,1,5,0,0,0,104,101,108,108,111]);
  assert.deepEqual(Array.from(decodeStringDataFrame(golden)),Array.from(golden));
  const {bridge,from,sender,receiver}=mk();
  const ptr=encode(from,0,'hello');
  bridge.sendPacked(sender,receiver,ptr);
  assert.equal(bridge.receiveNext(receiver,()=>{}).status,'HANDLER_RETURNED');
});

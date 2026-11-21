// The actual browser actor-worker source is evaluated in an isolated Node VM
// with a *mock* Wasm module; this tests the protocol and capability boundary,
// NOT real Rust compilation or guest-Wasm execution.
import test from 'node:test';
import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
import vm from 'node:vm';

const workerCode=readFileSync(new URL('./browser/actor-worker.mjs',import.meta.url),'utf8');

function createChild({moduleImports=[]}={}) {
  const messages=[];
  let closed=0;
  let privilegedImports;
  const self={
    onmessage:null,
    postMessage:value=>messages.push(value),
    close:()=>{closed++}
  };
  class MockModule {
    constructor(bytes){this.bytes=bytes}
    static imports(){return moduleImports}
  }
  class MockInstance {
    constructor(_module,ports){
      privilegedImports=ports.atto;
      this.exports={atto_actor_receive_3:value=>value+1n,panic_code:{value:0}};
    }
  }
  const WebAssembly={
    validate:bytes=>bytes instanceof Uint8Array&&bytes.length>0,
    Module:MockModule,
    Instance:MockInstance
  };
  vm.runInNewContext(workerCode,{self,WebAssembly,Uint8Array,Number,BigInt,Error,String}, {timeout:1000});
  const send=data=>self.onmessage({data});
  return {send,messages,get closed(){return closed},get privilegedImports(){return privilegedImports}};
}

const init=(incarnation=7)=>({
  kind:'init',actorId:3,incarnation,wasm:new Uint8Array([0,97,115,109])
});

test('initialized actor emits incarnation-bound READY, DATA reply, and DONE',()=>{
  const w=createChild();
  w.send(init(41));
  assert.equal(w.messages[0].kind,'ready');
  assert.equal(w.messages[0].incarnation,41);
  w.send({kind:'message',incarnation:41,value:9n});
  assert.equal(w.messages[1].kind,'reply');
  assert.equal(w.messages[1].incarnation,41);
  assert.equal(w.messages[1].value,10n);
  w.send({kind:'close',incarnation:41});
  assert.equal(w.messages[2].kind,'done');
  assert.equal(w.messages[2].incarnation,41);
  assert.equal(w.messages[2].processed,1);
  assert.equal(w.closed,1);
});

test('actor worker rejects stale DATA identity before invoking the guest',()=>{
  const w=createChild();
  w.send(init(101));
  w.send({kind:'message',incarnation:100,value:20n});
  assert.equal(w.messages.at(-1).kind,'error');
  assert.equal(w.messages.at(-1).incarnation,101);
  assert.equal(w.messages.at(-1).message,'E_ACTOR_INCAR_MISMATCH');
  assert.equal(w.messages.filter(x=>x.kind==='reply').length,0);
  assert.equal(w.closed,1);
});

test('actor worker rejects stale close and does not fabricate DONE',()=>{
  const w=createChild();
  w.send(init(102));
  w.send({kind:'close',incarnation:103});
  assert.equal(w.messages.at(-1).message,'E_ACTOR_INCAR_MISMATCH');
  assert.equal(w.messages.filter(x=>x.kind==='done').length,0);
  assert.equal(w.closed,1);
});

test('missing or invalid incarnation cannot publish READY',()=>{
  const missing=createChild();
  const without=init();
  delete without.incarnation;
  missing.send(without);
  assert.equal(missing.messages.at(-1).message,'E_ACTOR_INCAR_INVALID');
  for(const bad of [null,0,-1,1.5,Number.MAX_SAFE_INTEGER+1,'2']) {
    const w=createChild();
    w.send(init(bad));
    assert.equal(w.messages.at(-1).kind,'error');
    assert.equal(w.messages.at(-1).message,'E_ACTOR_INCAR_INVALID');
    assert.equal(w.messages.filter(x=>x.kind==='ready').length,0);
  }
});

test('child import links actor_ready but denies its use by actor guest',()=>{
  const w=createChild({moduleImports:[{module:'atto',name:'actor_ready'}]});
  w.send(init());
  assert.equal(w.messages.at(-1).kind,'ready');
  assert.throws(()=>w.privilegedImports.actor_ready(1),/E_ACTOR_NESTED_READY/);
});

test('unexpected import outside strict Atto actor ABI denies initialization',()=>{
  const w=createChild({moduleImports:[{module:'untrusted',name:'actor_ready'}]});
  w.send(init());
  assert.equal(w.messages.at(-1).kind,'error');
  assert.equal(w.messages.at(-1).message,'E_ACTOR_IMPORT');
  assert.equal(w.messages.filter(x=>x.kind==='ready').length,0);
});

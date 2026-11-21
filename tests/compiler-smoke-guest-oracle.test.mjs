// Source-free generated modules exercise oracle admission, NOT Rust compilation.
import {test} from 'node:test';
import assert from 'node:assert/strict';
import {verifyScalarGuestValue,verifyCheckedPanicGuest} from '../scripts/compiler-smoke-guest-oracle.mjs';

function uleb(v) {
  const b=[];do {let octet=v&127;v=Math.floor(v/128);if(v)octet|=128;b.push(octet)} while(v);return b;
}
function sleb(v) {
  let n=BigInt(v);const out=[];
  while(true){let b=Number(n&127n);n >>=7n;
    const done=(n===0n&&(b&64)===0)||(n===-1n&&(b&64)!==0);
    if(!done)b|=128;out.push(b);if(done)return out;
  }
}
const enc=new TextEncoder();
function textBytes(s){const b=[...enc.encode(s)];return [...uleb(b.length),...b]}
function section(id,bytes){return [id,...uleb(bytes.length),...bytes]}
function syntheticGuest({value=42n,panic=0,trap=false,exportMemory=true,declareMemory=true,exportPanic=true,returnI32=false,trapCode=1}={}){
  const type=section(1,[1,96,0,1,returnI32?127:126]);
  const func=section(3,[1,0]);
  const memory=declareMemory?section(5,[1,0,1]):[];
  const global=section(6,[1,127,1,65,...sleb(panic),11]);
  const symbols=[[...textBytes('main'),0,0]];
  if(exportPanic)symbols.push([...textBytes('panic_code'),3,0]);
  if(exportMemory)symbols.push([...textBytes('memory'),2,0]);
  const exports=section(7,[...uleb(symbols.length),...symbols.flat()]);
  const instructions=trap?[65,...sleb(trapCode),36,0,0,11]:[returnI32?65:66,...sleb(value),11];
  const body=[0,...instructions];
  const code=section(10,[1,...uleb(body.length),...body]);
  return Uint8Array.from([0,97,115,109,1,0,0,0,...type,...func,...memory,...global,...exports,...code]);
}
test('real Wasm instance yields exact signed i64 beyond JS safe integers',()=>{
  const value=-9007199254740993n;
  assert.equal(verifyScalarGuestValue(syntheticGuest({value}),value),value);
  assert.equal(verifyScalarGuestValue(syntheticGuest({value:55n}),55n),55n);
});
test('rejects missing required guest ABI exports',()=>{
  assert.throws(()=>verifyScalarGuestValue(syntheticGuest({exportPanic:false}),42n),/panic_code export/);
  assert.throws(()=>verifyScalarGuestValue(syntheticGuest({returnI32:true}),42n),/signed Wasm i64/);
});
test('admits compiler-generated scalar and checked-panic Wasm without linear memory',()=>{
  const memoryless={exportMemory:false,declareMemory:false};
  assert.equal(verifyScalarGuestValue(syntheticGuest(memoryless),42n),42n);
  assert.equal(verifyCheckedPanicGuest(syntheticGuest({...memoryless,trap:true,trapCode:1}),1),1);
});
test('rejects forged positive guest output with nonzero initial panic',()=>{
  assert.throws(()=>verifyScalarGuestValue(syntheticGuest({panic:1}),42n),/initialize to zero/);
  assert.throws(()=>verifyScalarGuestValue(syntheticGuest({value:7n}),42n),/unexpected compiled guest return value/);
});
test('rejects a malformed, non-Wasm, or oversized artifact',()=>{
  assert.throws(()=>verifyScalarGuestValue(new Uint8Array(8),42n),/valid WebAssembly/);
  assert.throws(()=>verifyScalarGuestValue(new Uint8Array(1024*1024+1),42n),/bounded Wasm size/);
  assert.throws(()=>verifyScalarGuestValue('a fake guest',42n),/Wasm bytes/);
});
test('rejects fixture expected result coerced into unsafe JavaScript number',()=>{
  assert.throws(()=>verifyScalarGuestValue(syntheticGuest(),42),/retain signed i64 precision/);
});
test('checked arithmetic panic requires a Wasm trap and a typed panic code',()=>{
  // A real Wasm main sets its exported panic global, then executes unreachable.
  const module=syntheticGuest({trap:true,trapCode:1});
  assert.equal(verifyCheckedPanicGuest(module,1),1);
  assert.throws(()=>verifyCheckedPanicGuest(syntheticGuest({trap:true,trapCode:2}),1),/exact language panic code/);
  assert.throws(()=>verifyCheckedPanicGuest(syntheticGuest(),1),/must trap/);
  assert.throws(()=>verifyCheckedPanicGuest(module,0),/explicit positive integer/);
});

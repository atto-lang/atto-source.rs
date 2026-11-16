// Security regression: exact Wasm signatures plus mandatory negotiated JSPI markers.
// This constructs real valid Wasm imports. Name-only admission is not sufficient.
import test from 'node:test';
import assert from 'node:assert/strict';
import {validateGuestImportABI} from '../runtime/browser/guest_import_admission_v2.mjs';

const U=n=>{const a=[]; do {let b=n&127;n>>>=7;if(n)b|=128;a.push(b)}while(n);return a};
const N=s=>{const a=[...new TextEncoder().encode(s)];return [...U(a.length),...a]};
const S=(id,b)=>[id,...U(b.length),...b];
const CUSTOM=(label,payload)=>S(0,[...N(label),payload]);
const magic=[0,97,115,109,1,0,0,0];
const i32=0x7f,i64=0x7e,f64=0x7c;
function moduleBytes(spec, options={}) {
  const {asyncMarker,channelMarker,injectMemory=false}=options;
  const typeSec=[...U(spec.length)];
  const impSec=[...U(spec.length+(injectMemory?1:0))];
  spec.forEach(([name,params,result,mod='atto'],idx)=>{
    typeSec.push(0x60,...U(params.length),...params,...U(result.length),...result);
    impSec.push(...N(mod),...N(name),0,...U(idx));
  });
  if(injectMemory) impSec.push(...N('atto'),...N('print_i64'),2,0,1);
  const bytes=[...magic,...S(1,typeSec),...S(2,impSec)];
  if(asyncMarker!==undefined)bytes.push(...CUSTOM('atto.async',asyncMarker));
  if(channelMarker!==undefined)bytes.push(...CUSTOM('atto.channel_int',channelMarker));
  return Uint8Array.from(bytes);
}
const readImports=[
 ['channel_int_nb_read_v1',[i32,i32],[i32]],
 ['channel_int_await_read_v1',[i32,i32],[i32]],
 ['cooperate',[],[]]
];
const verified=(bytes,policy)=>{
  assert.ok(WebAssembly.validate(bytes),'test must construct a real valid Wasm binary');
  const mod=new WebAssembly.Module(bytes);
  return validateGuestImportABI(bytes,mod,policy);
};
const rejects=(bytes,policy,reason)=>assert.throws(()=>verified(bytes,policy),reason);

test('channel-v1 genuine i32 ticket/JSPI imports require both immutable markers',()=>{
 const wasm=moduleBytes(readImports,{asyncMarker:2,channelMarker:1});
 assert.equal(verified(wasm,'channel-int-v1').length,3);
});
test('browser-basic denies channel imports even with valid markers',()=>{
 rejects(moduleBytes(readImports,{asyncMarker:2,channelMarker:1}),
         'browser-basic-v1',/not granted/);
});
test('ordinary async Worker cannot receive Channel<int> capabilities',()=>{
 rejects(moduleBytes(readImports,{asyncMarker:2,channelMarker:1}),
         'browser-async-v2',/not granted/);
});
test('channel-v1 denies missing and incorrect negotiated marker versions',()=>{
 for(const options of [{asyncMarker:2},{channelMarker:1},{asyncMarker:1,channelMarker:1},
   {asyncMarker:2,channelMarker:2},{}]){
  rejects(moduleBytes(readImports,options),'channel-int-v1',/marker/);
 }
});
test('wrong parameter/result widths fail closed even with correct JSPI marker',()=>{
 for(const bad of [
  ['channel_int_nb_read_v1',[i64,i32],[i32]],
  ['channel_int_nb_read_v1',[i32,i32],[i64]],
  ['channel_int_nb_read_v1',[i32],[i32]],
  ['channel_int_await_read_v1',[i32,i64],[i32]],
  ['cooperate',[],[i32]]
 ]) {
  const imports=readImports.map(x=>x[0]===bad[0]?bad:x);
  rejects(moduleBytes(imports,{asyncMarker:2,channelMarker:1}),
          'channel-int-v1',/signature mismatch/);
 }
});
test('deny foreign modules, generic host memory and surprising imports',()=>{
 rejects(moduleBytes([['evil',[i32],[],'env']]),'browser-async-v2',/not granted/);
 rejects(moduleBytes([['task_start',[i32],[i32], 'env']],{asyncMarker:2}),
         'browser-async-v2',/not granted/);
 rejects(moduleBytes(readImports,{asyncMarker:2,channelMarker:1,injectMemory:true}),
         'channel-int-v1',/non-function/);
 rejects(moduleBytes([...readImports,['actor_spawn',[i32],[i32]]],{asyncMarker:2,channelMarker:1}),
         'channel-int-v1',/not granted/);
});
test('async v2 admits the actual checked effect ABI, not channel-specific calls',()=>{
 const imports=[
  ['sleep_ms',[i64],[i64]],['barrier',[i64],[i64]],
  ['task_start',[i32],[i32]],['task_await',[i32],[i64]],['cooperate',[],[]]
 ];
 assert.equal(verified(moduleBytes(imports,{asyncMarker:2}),'browser-async-v2').length,5);
 rejects(moduleBytes(imports),'browser-async-v2',/missing async v2 marker/);
});
test('base host admits only reviewed simple signatures',()=>{
 assert.equal(verified(moduleBytes([['print_i64',[i64],[]]]),'browser-basic-v1').length,1);
 rejects(moduleBytes([['print_i64',[i32],[]]]),'browser-basic-v1',/signature mismatch/);
});
test('missing expected channel await import cannot be disguised as a valid profile',()=>{
 rejects(moduleBytes([readImports[0],readImports[2]],{asyncMarker:2,channelMarker:1}),
         'channel-int-v1',/mandatory channel JSPI signatures/);
});
test('JSPI profile cannot be selected without a validated WebAssembly.Module',()=>{
 const wasm=moduleBytes(readImports,{asyncMarker:2,channelMarker:1});
 assert.throws(()=>validateGuestImportABI(wasm,null,'channel-int-v1'),/validated WebAssembly.Module/);
});
test('unknown host policy refuses all imports, including harmless stdio',()=>{
 const wasm=moduleBytes([['print_newline',[],[]]]);
 assert.throws(()=>validateGuestImportABI(wasm,new WebAssembly.Module(wasm),'ffi-unsafe'),/unsupported host policy/);
});

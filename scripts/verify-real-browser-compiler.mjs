#!/usr/bin/env node
// Actual native attoc + wasm32 Rust compiler + guest execution. No mock compiler.
import assert from 'node:assert/strict';
import {createHash} from 'node:crypto';
import {spawnSync} from 'node:child_process';
import {existsSync, mkdtempSync, readFileSync, rmSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {dirname, join, resolve} from 'node:path';
import {fileURLToPath} from 'node:url';
const root=resolve(dirname(fileURLToPath(import.meta.url)),'..');
const native=resolve(process.env.ATTOC||join(root,'target/release/attoc'));
const compiler=resolve(process.env.ATTO_WASM_COMPILER||
  join(root,'target/wasm32-unknown-unknown/release/atto_compiler.wasm'));
const sha=x=>createHash('sha256').update(x).digest('hex');
const decoder=new TextDecoder('utf-8',{fatal:true});
// Explicit contract-backed, independent positive and negative programs.
// Do not let a repeated success fixture masquerade as broader guest coverage.
const tests=[
  ['pass','01-answer.atto',42n],
  ['pass','02-spec-ch16-add.atto',42n],
  ['pass','03-arrow-return.atto',42n],
  ['pass','04-binding-forms.atto',52n],
  ['pass','06-precedence.atto',120n],
  ['pass','19-recursion-fib.atto',55n],
  ['fail','01-val-reassign.atto','E_IMMUTABLE'],
  ['fail','02-param-reassign.atto','E_IMMUTABLE'],
  ['fail','03-unknown-name.atto','E_UNBOUND'],
  ['fail','09-call-arity.atto','E_ARITY'],
  ['fail','23-no-main.atto','E_ENTRY']
];
assert.equal(new Set(tests.map(([kind,name])=>kind+'/'+name)).size,tests.length,
  'smoke matrix contains duplicate source fixtures');
function file(path){assert.ok(existsSync(path),'missing real compiler/fixture: '+path);return readFileSync(path)}
function wasmInstance(){
  const bytes=file(compiler);
  // The same public Pages release gate must be enforced by native-vs-wasm
  // compiler parity tests. A larger candidate is not deployable to /play/.
  assert.ok(bytes.length <= 1048576,'E_COMPILER_SIZE: compiler exceeds public 1 MiB budget');
  assert.ok(WebAssembly.validate(bytes),'compiler binary is not Wasm');
  const mod=new WebAssembly.Module(bytes);
  assert.deepEqual(WebAssembly.Module.imports(mod),[],'compiler cannot import host capabilities');
  const api=new WebAssembly.Instance(mod,{}).exports;
  for(const name of ['memory','abi_version','input_ptr','input_capacity','compile','result_ptr','result_length','error_ptr'])
    assert.ok(name in api,'missing compiler ABI '+name);
  assert.equal(api.abi_version(),2,'compiler ABI v2 required');
  assert.ok(api.input_capacity()>0&&api.input_capacity()<=65536,'unexpected source capacity');
  return {api,digest:sha(bytes)};
}
function wasmCompile(api,source){
  const bytes=new TextEncoder().encode(source),ptr=api.input_ptr();
  assert.ok(bytes.length<=api.input_capacity(),'fixture too large');
  assert.ok(ptr>=0&&ptr+bytes.length<=api.memory.buffer.byteLength,'source buffer bounds');
  new Uint8Array(api.memory.buffer,ptr,bytes.length).set(bytes);
  const status=api.compile(bytes.length);
  assert.ok(status===0||status===1,'unexpected compile return status');
  if(status){
    assert.equal(api.result_length(),0,'compiler published rejected guest bytes');
    const mem=new Uint8Array(api.memory.buffer),start=api.error_ptr();
    assert.ok(start>=0&&start<mem.length,'diagnostic bounds');
    let end=start;while(end<mem.length&&end<start+512&&mem[end]!==0)end++;
    assert.ok(end<start+512,'unterminated diagnostic');
    return {ok:false,message:decoder.decode(mem.subarray(start,end))};
  }
  const ptrOut=api.result_ptr(),len=api.result_length();
  assert.ok(len>=8&&len<=1048576&&ptrOut>=0&&ptrOut+len<=api.memory.buffer.byteLength,'guest result bounds');
  const guest=new Uint8Array(api.memory.buffer,ptrOut,len).slice();
  assert.ok(WebAssembly.validate(guest),'compiler emitted invalid guest');
  return {ok:true,guest};
}
function guestValue(bytes){
  const mod=new WebAssembly.Module(bytes);
  assert.deepEqual(WebAssembly.Module.imports(mod),[],'scalar guest unexpectedly imports');
  const exp=new WebAssembly.Instance(mod,{}).exports;
  assert.equal(typeof exp.main,'function','guest main missing');
  const n=exp.main();assert.equal(typeof n,'bigint','main must return i64');
  if('panic_code' in exp)assert.ok(exp.panic_code instanceof WebAssembly.Global&&
    (exp.panic_code.value===0||exp.panic_code.value===0n),'guest panicked');
  return n;
}
const tmp=mkdtempSync(join(tmpdir(),'atto-real-rust-wasm-'));
try{
  const {api,digest}=wasmInstance(),nativeHash=sha(file(native)),rows=[];
  for(let i=0;i<tests.length;i++){
    const [kind,name,oracle]=tests[i],src=join(root,'tests/fixtures',kind,name);
    const content=file(src),output=join(tmp,String(i)+'.wasm');
    const source=decoder.decode(content);
    const annotation=source.match(kind==='pass'
      ? /^\\/\\/ expect-exit:\\s*(\\d+)\\s*$/m
      : /^\\/\\/ expect-error:\\s*(E_[A-Z0-9_]+)\\s*$/m);
    assert.ok(annotation,'missing fixture expectation: '+name);
    assert.equal(annotation[1],String(oracle),'smoke expectation drift: '+name);
    const cp=spawnSync(native,[src,'-o',output],{cwd:root,encoding:'utf8',timeout:20000,maxBuffer:1048576,shell:false});
    assert.ifError(cp.error);assert.equal(cp.signal,null,'native compiler terminated by signal');
    const wasm=wasmCompile(api,source);
    if(kind==='pass'){
      assert.equal(cp.status,0,'native rejected '+name+': '+cp.stderr);
      assert.equal(wasm.ok,true,'wasm32 compiler rejected '+name);
      const binary=file(output);
      assert.deepEqual(wasm.guest,binary,'native/wasm32 compiler guest bytes differ');
      assert.equal(guestValue(binary),oracle,'native guest result');
      assert.equal(guestValue(wasm.guest),oracle,'wasm32 guest result');
      rows.push({fixture:name,result:'REAL_GUEST',value:String(oracle),source_sha256:sha(content),guest_sha256:sha(binary)});
    }else{
      assert.equal(cp.status,1,'native accepted invalid '+name);
      assert.ok(!existsSync(output),'rejected native source emitted output');
      assert.ok(cp.stderr.includes(oracle),'native missing expected diagnostic '+oracle);
      assert.equal(wasm.ok,false,'wasm32 accepted invalid '+name);
      assert.ok(wasm.message.includes(oracle),'wasm32 missing expected diagnostic '+oracle);
      rows.push({fixture:name,result:'REAL_REJECTION',code:oracle,source_sha256:sha(content),no_artifact:true});
    }
  }
  console.log(JSON.stringify({schema:'atto.rust_wasm.real_smoke.v1',native_sha256:nativeHash,wasm_compiler_sha256:digest,rows},null,2));
}finally{rmSync(tmp,{recursive:true,force:true})}

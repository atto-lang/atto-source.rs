// Exact-source native attoc vs real wasm32 compiler import parity.
// Both compilers must independently lower the same backend-owned .atto
// std/testing_scalar into byte-identical guest Wasm and real guest results.
// No host source interpretation, fake receipts, or compiler fallback.
import assert from 'node:assert/strict';
import {createHash} from 'node:crypto';
import {spawnSync} from 'node:child_process';
import {existsSync,readFileSync,writeFileSync,mkdtempSync,rmSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {join,resolve,dirname} from 'node:path';
import {fileURLToPath} from 'node:url';
import {SCALAR_TEST_CASES,SCALAR_TEST_NEGATIVES,scalarTestProgram} from './testing-scalar-vectors.mjs';

const here=dirname(fileURLToPath(import.meta.url));
const compilerPath=resolve(here,'../target/wasm32-unknown-unknown/release/atto_compiler.wasm');
const nativePath=process.env.ATTOC??resolve(here,'../target/release/attoc');
assert.ok(existsSync(compilerPath),'real wasm32 compiler artifact is required');
assert.ok(existsSync(nativePath),'real native attoc compiler is required');
const compilerBytes=readFileSync(compilerPath);
assert.ok(WebAssembly.validate(compilerBytes),'compiler artifact is invalid Wasm');
const module=new WebAssembly.Module(compilerBytes);
assert.deepEqual(WebAssembly.Module.imports(module),[],'compiler must have zero host imports');
const api=new WebAssembly.Instance(module,{}).exports;
assert.equal(api.abi_version(),2,'incorrect browser compiler ABI');
const encoder=new TextEncoder();
const decoder=new TextDecoder('utf-8',{fatal:true});
const sha256=data=>createHash('sha256').update(data).digest('hex');

function browserCompile(source){
  const input=encoder.encode(source);
  assert.ok(input.length>0&&input.length<=api.input_capacity(),'browser input bounded');
  const start=api.input_ptr();
  assert.ok(start>=0&&start+input.length<=api.memory.buffer.byteLength,'input memory bounded');
  new Uint8Array(api.memory.buffer,start,input.length).set(input);
  const status=api.compile(input.length);
  if(status!==0){
    assert.equal(api.result_length(),0,'rejected compiler source emitted Wasm');
    const mem=new Uint8Array(api.memory.buffer);
    const ptr=api.error_ptr();
    assert.ok(ptr>=0&&ptr<mem.byteLength,'invalid browser diagnostic pointer');
    let end=ptr;
    while(end<mem.length&&end<ptr+512&&mem[end]!==0)end++;
    return {ok:false,error:decoder.decode(mem.subarray(ptr,end))};
  }
  const offset=api.result_ptr(),length=api.result_length();
  assert.ok(length>=8&&length<=1048576&&offset>=0&&offset+length<=api.memory.buffer.byteLength,
    'browser compiler output exceeded guest memory budget');
  const bytes=new Uint8Array(api.memory.buffer,offset,length).slice();
  assert.ok(WebAssembly.validate(bytes),'browser compiler returned malformed guest');
  return {ok:true,wasm:bytes};
}
function runGuest(bytes,id){
  const mod=new WebAssembly.Module(bytes);
  assert.deepEqual(WebAssembly.Module.imports(mod),[],id+': imported guest capabilities');
  const guest=new WebAssembly.Instance(mod,{});
  assert.equal(typeof guest.exports.main,'function',id+': guest main missing');
  const result=guest.exports.main();
  assert.equal(guest.exports.panic_code.value,0,id+': guest panicked');
  return result;
}
function nativeCompile(input,output){
  return spawnSync(nativePath,[input,'-o',output],{encoding:'utf8',timeout:30000,maxBuffer:1<<20});
}

const work=mkdtempSync(join(tmpdir(),'atto-testing-wasm32-parity-'));
const receipts=[];
try {
  for(const [id,expression,expected] of SCALAR_TEST_CASES){
    const source=scalarTestProgram(expression);
    const input=join(work,id+'.atto');
    const output=join(work,id+'.wasm');
    writeFileSync(input,source);
    const browser=browserCompile(source);
    assert.equal(browser.ok,true,id+': wasm32 compiler rejected import: '+browser.error);
    const native=nativeCompile(input,output);
    assert.equal(native.error,undefined,id+': native compiler launch error');
    assert.equal(native.status,0,id+': native compiler rejected import: '+native.stderr);
    assert.ok(existsSync(output),id+': native compiler produced no Wasm');
    const bytes=readFileSync(output);
    assert.deepEqual(bytes,Buffer.from(browser.wasm),
      id+': different compiler targets emitted different guest Wasm bytes');
    assert.equal(runGuest(bytes,id),expected,id+': native output guest value mismatch');
    assert.equal(runGuest(browser.wasm,id),expected,id+': wasm32 output guest value mismatch');
    receipts.push({id,source_sha256:sha256(source),guest_sha256:sha256(bytes),
      expected:String(expected),native_compiler:true,browser_compiler:true,
      native_guest_executed:true,browser_guest_executed:true});
  }
  for(const [id,source] of SCALAR_TEST_NEGATIVES){
    const input=join(work,'reject-'+id+'.atto');
    const output=join(work,'reject-'+id+'.wasm');
    writeFileSync(input,source);
    const browser=browserCompile(source);
    assert.equal(browser.ok,false,id+': wasm32 compiler admitted negative import');
    assert.match(browser.error,/E_[A-Z0-9_]+/,id+': browser compiler missing error code');
    const native=nativeCompile(input,output);
    assert.equal(native.error,undefined,id+': native compiler launch error');
    assert.ok(native.status!==0&&native.status!==null,id+': native compiler admitted negative import');
    assert.equal(existsSync(output),false,id+': rejected native import emitted guest artifact');
    assert.match(native.stderr,/E_[A-Z0-9_]+/,id+': native compiler missing error code');
    receipts.push({id,source_sha256:sha256(source),native_rejected:true,
      browser_rejected:true,no_native_artifact:true,no_browser_artifact:true});
  }
  assert.equal(receipts.length,SCALAR_TEST_CASES.length+SCALAR_TEST_NEGATIVES.length);
  console.log(JSON.stringify({
    schema:'atto.testing-scalar.native-wasm32-import-parity.v1',
    native_compiler_sha256:sha256(readFileSync(nativePath)),
    wasm32_compiler_sha256:sha256(compilerBytes),
    library_sha256:sha256(readFileSync(resolve(here,'../stdlib/testing_scalar.atto'))),
    positive_cases:SCALAR_TEST_CASES.length,negative_cases:SCALAR_TEST_NEGATIVES.length,
    results:receipts,
  }));
} finally {rmSync(work,{recursive:true,force:true});}

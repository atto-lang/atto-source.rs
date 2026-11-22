// Actual Rust attoc -> WebAssembly bytes -> executed guest; no interpreter substitution.
import assert from 'node:assert/strict';
import {spawnSync} from 'node:child_process';
import {mkdtemp,readFile,rm,stat} from 'node:fs/promises';
import {tmpdir} from 'node:os';
import {join} from 'node:path';
import {fileURLToPath} from 'node:url';
const compiler=process.env.ATTOC ?? fileURLToPath(new URL('../target/debug/attoc',import.meta.url));
const fixtures=fileURLToPath(new URL('./fixtures/core-types-v1/',import.meta.url));
const work=await mkdtemp(join(tmpdir(),'core-types-rs-'));
try{
  const source=join(fixtures,'edge-positive.atto'),out=join(work,'positive.wasm');
  const positive=spawnSync(compiler,[source,'-o',out],{encoding:'utf8',timeout:120000});
  assert.equal(positive.status,0,'real attoc failed to link core/types: '+positive.stderr);
  const wasm=await readFile(out);
  assert.ok(WebAssembly.validate(wasm),'compiler emitted invalid WebAssembly');
  const {instance}=await WebAssembly.instantiate(wasm,{atto:{}});
  assert.equal(typeof instance.exports.main,'function','compiled main missing');
  assert.equal(instance.exports.main(),0n,'all range checks must execute to exit 0');
  if(instance.exports.panic_code)assert.equal(instance.exports.panic_code.value,0);
  console.log('CORE_TYPES_RS_WASM_GUEST=EXECUTED_EXIT_0');
  for(const name of ['void-value-negative.atto','void-return-negative.atto',
    'bool-truthiness-negative.atto','i64-literal-overflow-negative.atto']){
    const rejected=join(work,name+'.wasm');
    const r=spawnSync(compiler,[join(fixtures,name),'-o',rejected],{encoding:'utf8',timeout:120000});
    assert.notEqual(r.status,0,name+' unexpectedly compiled');
    assert.ok(r.stderr.trim().length>0,name+' missing compiler diagnostic');
    assert.doesNotMatch(r.stderr,/panicked at|thread 'main' panicked|internal compiler error/i,
      name+' failed due to compiler crash, not diagnosed source error');
    await assert.rejects(stat(rejected),{code:'ENOENT'},name+' emitted artifact after rejection');
    console.log('CORE_TYPES_RS_COMPILE_REJECTED='+name);
  }
}finally{await rm(work,{recursive:true,force:true});}

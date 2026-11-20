// Real source-compiled .atto -> native attoc -> guest WebAssembly smoke.
// This proves only the closed-world ready Mutex<int> slice. It does NOT
// exercise contended async scheduling, TenantVM isolation or general Mutex<T>.
// Requires ATTOC or ATTOC_BIN and an exact ATTO_SOURCE_SHA in CI.
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { existsSync, mkdtempSync, readFileSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, isAbsolute, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const here=dirname(fileURLToPath(import.meta.url));
const root=resolve(here,'mutex_fast_guest_fixtures');
const bin=resolve(process.env.ATTOC_BIN||process.env.ATTOC||'');
assert.ok((process.env.ATTOC_BIN||process.env.ATTOC)&&existsSync(bin),
  'a real native attoc binary must be provided');
const sha=process.env.ATTO_SOURCE_SHA||'';
assert.match(sha,/^[a-f0-9]{40}$/,'exact source SHA required');
const digest=buf=>createHash('sha256').update(buf).digest('hex');
const binarySha=digest(readFileSync(bin));
const temp=mkdtempSync(join(tmpdir(),'atto-mutex-fast-'));
const results=[];
function compile(id,expectError) {
  const source=resolve(root,id+'.atto');
  assert.ok(source.startsWith(root+'/'),'fixture must stay in the source subtree');
  const out=join(temp,id+'.wasm');
  const run=spawnSync(bin,[source,'-o',out],{encoding:'utf8',timeout:45000,maxBuffer:65536});
  if(run.error)throw run.error;
  const exists=existsSync(out);
  const diagnostic=(run.stderr.match(/\bE_[A-Z0-9_]+\b/)||[])[0]||null;
  if(expectError) {
    assert.notEqual(run.status,0,id+': expected a compiler rejection');
    assert.equal(diagnostic,expectError,id+': wrong diagnostic '+run.stderr);
    assert.equal(exists,false,id+': compiler left an executable Wasm artifact');
    results.push({id,kind:'negative',diagnostic,guest_artifact:false,status:'PASS'});
    return;
  }
  assert.equal(run.status,0,id+': compiler failed: '+run.stderr);
  assert.ok(exists,id+': no emitted Wasm');
  const bytes=readFileSync(out);
  assert.ok(bytes.length>8&&bytes.length<1<<20,id+': wrong artifact size');
  assert.ok(WebAssembly.validate(bytes),id+': compiler emitted invalid guest Wasm');
  const mod=new WebAssembly.Module(bytes);
  const imports=WebAssembly.Module.imports(mod);
  for(const i of imports){
    assert.equal(i.module,'atto',id+': unsupported guest ABI module');
    assert.equal(i.kind,'function',id+': no ambient globals allowed');
    assert.ok(['print_i64','print_newline'].includes(i.name),
      id+': missing verified host scheduler/permission ABI '+i.name);
  }
  let stdout='';
  const instance=new WebAssembly.Instance(mod,{atto:{
    print_i64(v){stdout+=String(v)},
    print_newline(){stdout+='\n'}
  }});
  assert.equal(typeof instance.exports.main,'function');
  const returnValue=instance.exports.main();
  assert.equal(typeof returnValue,'bigint','main must return i64');
  assert.equal(instance.exports.panic_code.value,0,'guest trapped/panicked');
  const result={id,kind:'real_compiled_wasm_guest',exit:Number(returnValue),stdout,
    wasm_sha256:digest(bytes),wasm_bytes:bytes.length,status:'PASS'};
  results.push(result);
  return result;
}
try {
  const one=compile('return-one');
  assert.deepEqual([one.exit,one.stdout],[1,'']);
  const twice=compile('return-twice');
  assert.deepEqual([twice.exit,twice.stdout],[2,'']);
  const forty=compile('return-forty');
  assert.deepEqual([forty.exit,forty.stdout],[40,'']);
  assert.notEqual(one.wasm_sha256,forty.wasm_sha256,
    'guest must incorporate different scalar source values');
  const explicit=compile('print-explicit');
  assert.deepEqual([explicit.exit,explicit.stdout],[0,'1\n']);
  const implicit=compile('print-implicit');
  assert.deepEqual([implicit.exit,implicit.stdout],[0,'1\n']);
  assert.equal(explicit.wasm_sha256,implicit.wasm_sha256,
    'contextual await must lower to the same guest bytes');
  const lazy=compile('lazy-unawaited');
  assert.deepEqual([lazy.exit,lazy.stdout],[0,'']);
  const neverDriven=compile('lazy-overflow-unexecuted');
  assert.deepEqual([neverDriven.exit,neverDriven.stdout],[0,'']);
  compile('negative-guard-ready','E_GUARD_SUSPEND');
  compile('negative-guard-escape','E_GUARD_ESCAPE');
  compile('negative-await-effect','E_AWAIT_EFFECT');
} finally {
  rmSync(temp,{recursive:true,force:true});
}
const receipt={schema:'atto.mutex-fast-guest.v1',profile:'single_task_uncontended_only',
  compiler_source_head:sha,compiler_binary_sha256:binarySha,
  cases:results,pass:results.length===10&&results.every(x=>x.status==='PASS'),
  excluded:['contended_async','tenant_isolation','general_task_runtime','exceptional_guard_cleanup']};
if(process.env.ATTO_RECEIPT_PATH){
  const {writeFileSync}=await import('node:fs');
  writeFileSync(process.env.ATTO_RECEIPT_PATH,JSON.stringify(receipt,null,2)+'\n');
}
console.log('ATTO_MUTEX_FAST_GUEST_RECEIPT='+JSON.stringify(receipt));
assert.ok(receipt.pass,'no guest parity promotion without real execution');

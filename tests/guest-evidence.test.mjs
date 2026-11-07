import test from 'node:test';
import assert from 'node:assert/strict';
import { sha256, makeEvidence } from './guest-evidence.mjs';
const compiler=Buffer.from([0,97,115,109,1,0,0,0,2,1,1]);
const input='pub fnc main(): int { stdio.println(42); return 42; }';
function valid() {
  return {
    repository:'atto-lang/atto-source.rs',
    sourceSha:'a'.repeat(40),treeSha:'b'.repeat(40),
    runId:42,runAttempt:2,jobId:123,nonce:'f'.repeat(32),
    compilerBytes:compiler,witnessStdout:'42\n',
    cases:[{case_id:'witness/stdio-42',source_sha256:sha256(Buffer.from(input)),
      guest_module_sha256:'d'.repeat(64),
      observed_stdout_sha256:sha256(Buffer.from('42\n')),
      outcome:{kind:'returned',value:'42'}}]
  };
}
test('source-free receipt matches canonical Rust guest schema',()=>{
  const x=makeEvidence(valid());
  const r=x.receipt;
  assert.deepEqual(Object.keys(r).sort(),
    ['version','repository','source_sha','tree_sha','case_sha256','backend','feature',
      'run_id','run_attempt','job_id','target','command','nonce','executed',
      'failed','exit_code','stdout_sha256'].sort());
  assert.equal(r.backend,'wasm');
  assert.equal(r.target,'guest');
  assert.equal(r.executed,1);
  assert.equal(r.failed,0);
  assert.equal(r.exit_code,0);
  assert.equal(r.stdout_sha256,sha256(x.stdoutBytes));
  assert.equal(r.case_sha256,sha256(x.fixtureBytes));
  assert.equal(JSON.parse(x.fixtureBytes).length,r.executed);
  assert.equal(JSON.parse(x.fixtureBytes)[0].compiler_sha256,sha256(compiler));
});
test('reject missing/stale run identity and invalid compiler bytes',()=>{
  for(const [key,value] of [
    ['repository','evil/../../repo'],['sourceSha','wrong'],['treeSha','wrong'],
    ['runId',0],['runAttempt',0],['jobId',0],['nonce','predictable'],
    ['compilerBytes',Buffer.from('pretend compiler')],['witnessStdout',''],
    ['witnessStdout','43\n']
  ]) assert.throws(()=>makeEvidence({...valid(),[key]:value}),key);
});
test('reject duplicate case IDs and missing executed guest artifacts',()=>{
  const v=valid(),c=v.cases[0];
  assert.throws(()=>makeEvidence({...v,cases:[]}));
  assert.throws(()=>makeEvidence({...v,cases:[c,c]}));
  assert.throws(()=>makeEvidence({...v,cases:[{...c,outcome:{kind:'host_only'}}]}));
  assert.throws(()=>makeEvidence({...v,cases:[{...c,guest_module_sha256:'not-a-hash'}]}));
});
test('fixture-set identity changes when witness output or module changes',()=>{
  const v=valid();
  const a=makeEvidence(v);
  const b=makeEvidence({...v,cases:[{...v.cases[0],guest_module_sha256:'e'.repeat(64)}]});
  assert.notEqual(a.receipt.case_sha256,b.receipt.case_sha256);
});

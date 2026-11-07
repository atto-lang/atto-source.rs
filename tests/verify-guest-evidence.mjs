// Independent second-process replay of the actual Wasm compiler + guest.
// This is a source-owned claim consistency check, NOT release certification.
import assert from 'node:assert/strict';
import {readFile} from 'node:fs/promises';
import {execFileSync} from 'node:child_process';
import {join} from 'node:path';
import {sha256} from './guest-evidence.mjs';

const [root,compilerPath]=process.argv.slice(2);
if(!root||!compilerPath||process.argv.length!==4)
  throw Error('usage: node tests/verify-guest-evidence.mjs CLAIM_DIR COMPILER_WASM');
const bounded=async(path,limit)=>{
  const data=await readFile(path);
  assert.ok(data.length>0&&data.length<=limit,'missing or oversized evidence');
  return data;
};
const receipt=JSON.parse((await bounded(join(root,'guest-receipt.json'),32768)).toString('utf8'));
const fixtures=await bounded(join(root,'fixture-set.json'),4*1024*1024);
const entries=JSON.parse(fixtures.toString('utf8'));
const stdout=await bounded(join(root,'guest-stdout.bin'),1048576);
const compilerBytes=await bounded(compilerPath,67108864);
const git=(...args)=>execFileSync('git',args,{encoding:'utf8'}).trim();
assert.deepEqual(Object.keys(receipt).sort(),
  ['version','repository','source_sha','tree_sha','case_sha256','backend','feature',
   'run_id','run_attempt','job_id','target','command','nonce','executed',
   'failed','exit_code','stdout_sha256'].sort());
assert.equal(receipt.version,1);
assert.equal(receipt.repository,process.env.GITHUB_REPOSITORY);
assert.equal(receipt.source_sha,git('rev-parse','HEAD'));
assert.equal(receipt.source_sha,process.env.ATTO_EVIDENCE_EXPECTED_HEAD);
assert.equal(receipt.tree_sha,git('rev-parse','HEAD^{tree}'));
assert.equal(receipt.run_id,Number(process.env.GITHUB_RUN_ID));
assert.equal(receipt.run_attempt,Number(process.env.GITHUB_RUN_ATTEMPT));
assert.equal(receipt.job_id,Number(process.env.ATTO_GUEST_JOB_ID));
assert.equal(receipt.nonce,process.env.ATTO_GUEST_NONCE);
assert.equal(receipt.backend,'wasm');
assert.equal(receipt.feature,'browser.compiler.guest.v1');
assert.equal(receipt.target,'guest');
assert.equal(receipt.command,'node tests/conformance.mjs');
assert.equal(receipt.failed,0);
assert.equal(receipt.exit_code,0);
assert.ok(Array.isArray(entries)&&entries.length>0&&entries.length<=1000);
assert.equal(receipt.executed,entries.length);
assert.equal(receipt.case_sha256,sha256(fixtures));
assert.equal(receipt.stdout_sha256,sha256(stdout));
assert.equal(stdout.toString('utf8'),'42\n');

const compilerHash=sha256(compilerBytes),seen=new Set();
for(const entry of entries) {
  assert.match(entry.case_id??'',/^(pass|panic|witness)\/[A-Za-z0-9._-]+$/);
  assert.ok(!seen.has(entry.case_id),'duplicate guest identifier');
  seen.add(entry.case_id);
  assert.equal(entry.compiler_sha256,compilerHash);
  for(const key of ['source_sha256','guest_module_sha256','observed_stdout_sha256'])
    assert.match(entry[key]??'',/^[0-9a-f]{64}$/);
  if(entry.case_id.startsWith('pass/')||entry.case_id.startsWith('panic/')) {
    const [kind,name]=entry.case_id.split('/');
    const source=await readFile(join('tests','fixtures',kind,name));
    assert.equal(sha256(source),entry.source_sha256,'stale fixture');
  }
}
const witness=entries.find(c=>c.case_id==='witness/stdio-42');
assert.ok(witness,'witness case absent');
const source='pub fnc main(): int { stdio.println(42); return 42; }';
assert.equal(witness.source_sha256,sha256(Buffer.from(source,'utf8')));
assert.deepEqual(witness.outcome,{kind:'returned',value:'42'});
assert.equal(witness.observed_stdout_sha256,sha256(stdout));
assert.ok(WebAssembly.validate(compilerBytes));
const cm=new WebAssembly.Module(compilerBytes);
assert.deepEqual(WebAssembly.Module.imports(cm),[],'compiler cannot import host capabilities');
const api=new WebAssembly.Instance(cm,{}).exports;
assert.equal(api.abi_version(),2);
const sourceBytes=Buffer.from(source,'utf8');
assert.ok(sourceBytes.length<=api.input_capacity());
new Uint8Array(api.memory.buffer,api.input_ptr(),sourceBytes.length).set(sourceBytes);
assert.equal(api.compile(sourceBytes.length),0,'actual compiler rejected witness');
const guestBytes=new Uint8Array(api.memory.buffer,api.result_ptr(),api.result_length()).slice();
assert.equal(sha256(guestBytes),witness.guest_module_sha256);
assert.ok(WebAssembly.validate(guestBytes));
const guestModule=new WebAssembly.Module(guestBytes);
const allowed=new Set(['print_i64','print_bool','print_str','print_newline']);
for(const imp of WebAssembly.Module.imports(guestModule)) {
  assert.equal(imp.module,'atto');
  assert.ok(allowed.has(imp.name));
}
let out='';
const guest=new WebAssembly.Instance(guestModule,{atto:{
  print_i64:x=>{out+=String(x);},
  print_bool:x=>{out+=x?'true':'false';},
  print_newline:()=>{out+='\n';},
  print_str:()=>{throw Error('unexpected guest import');}
}}).exports;
assert.equal(guest.main(),42n);
assert.equal(out,stdout.toString('utf8'));
console.log(JSON.stringify({
  status:'NOT_CERTIFIED',
  check:'SECOND_PROCESS_ACTUAL_WASM_GUEST_EXECUTED',
  compiler_sha256:compilerHash,
  guest_sha256:witness.guest_module_sha256,
  fixture_set_sha256:receipt.case_sha256,
  source_sha:receipt.source_sha,
  observed_main_result:'42',
  observed_stdout_sha256:sha256(Buffer.from(out))
}));

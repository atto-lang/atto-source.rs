// Source-free, machine-checkable *claim* produced only after actual Wasm
// guest tests complete. This is NOT a signed attestation or release certificate.
import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { mkdir, writeFile } from 'node:fs/promises';
import { resolve } from 'node:path';

const HEX40=/^[0-9a-f]{40}$/;
const HEX64=/^[0-9a-f]{64}$/;
const CASE=/^(pass|panic|witness)\/[a-zA-Z0-9._-]+$/;

export function sha256(bytes) {
  return createHash('sha256').update(bytes).digest('hex');
}
function safeInteger(n) {
  return Number.isSafeInteger(n) && n > 0;
}
function checkCases(cases) {
  assert.ok(Array.isArray(cases) && cases.length > 0 && cases.length <= 1000,
    'nonempty, bounded executed guest corpus required');
  const seen=new Set();
  for(const c of cases) {
    assert.ok(c && typeof c === 'object', 'case record required');
    assert.ok(CASE.test(c.case_id), 'unsafe guest case identifier');
    assert.ok(!seen.has(c.case_id), 'duplicate executed guest case');
    seen.add(c.case_id);
    for (const field of ['source_sha256','guest_module_sha256','observed_stdout_sha256']) {
      assert.match(c[field] ?? '',HEX64,field);
    }
    assert.ok(
      c.outcome?.kind === 'returned' && /^-?[0-9]+$/.test(c.outcome.value ?? '')
      || c.outcome?.kind === 'panic' && Number.isInteger(c.outcome.code) && c.outcome.code > 0,
      'guest result must be explicit exit value or panic code');
  }
}
export function makeEvidence({repository,sourceSha,treeSha,runId,runAttempt,
  jobId,nonce,compilerBytes,cases,witnessStdout}) {
  assert.match(repository??'',/^[a-zA-Z0-9-]{1,39}\/[a-zA-Z0-9_.-]{1,100}$/);
  assert.match(sourceSha??'',HEX40);
  assert.match(treeSha??'',HEX40);
  assert.ok(safeInteger(runId) && safeInteger(runAttempt) && safeInteger(jobId),
    'trusted numeric GitHub run/attempt/job identity required');
  assert.match(nonce??'',/^[a-f0-9]{32,128}$/);
  assert.ok(Buffer.isBuffer(compilerBytes) && compilerBytes.length > 8,
    'actual compiler Wasm bytes required');
  assert.ok(Buffer.from(compilerBytes).subarray(0,8).equals(
    Buffer.from([0,97,115,109,1,0,0,0])), 'actual WebAssembly compiler magic required');
  assert.ok(typeof witnessStdout === 'string' && witnessStdout === '42\n',
    'literal guest stdout witness did not execute correctly');
  checkCases(cases);
  const compilerHash=sha256(compilerBytes);
  const sorted=[...cases].sort((a,b)=>a.case_id.localeCompare(b.case_id));
  const annotated=sorted.map(c=>({
    case_id:c.case_id,
    source_sha256:c.source_sha256,
    guest_module_sha256:c.guest_module_sha256,
    compiler_sha256:compilerHash,
    observed_stdout_sha256:c.observed_stdout_sha256,
    outcome:c.outcome
  }));
  const fixtureBytes=Buffer.from(JSON.stringify(annotated)+'\n');
  if (fixtureBytes.length > 4*1024*1024) throw Error('fixture evidence exceeds cap');
  const stdoutBytes=Buffer.from(witnessStdout,'utf8');
  const receipt={
    version:1,
    repository,
    source_sha:sourceSha,
    tree_sha:treeSha,
    case_sha256:sha256(fixtureBytes),
    backend:'wasm',
    feature:'browser.compiler.guest.v1',
    run_id:runId,
    run_attempt:runAttempt,
    job_id:jobId,
    target:'guest',
    command:'node tests/conformance.mjs',
    nonce,
    executed:annotated.length,
    failed:0,
    exit_code:0,
    stdout_sha256:sha256(stdoutBytes),
  };
  return { receipt, fixtureBytes, stdoutBytes };
}
// The caller must select an explicit *local* output directory. No source or
// secret environment contents are included in the artifact.
export async function writeEvidence(outputDir,params) {
  const output=resolve(outputDir);
  const {receipt,fixtureBytes,stdoutBytes}=makeEvidence(params);
  await mkdir(output,{recursive:true,mode:0o700});
  await Promise.all([
    writeFile(resolve(output,'guest-receipt.json'),JSON.stringify(receipt)+'\n',{flag:'wx',mode:0o600}),
    writeFile(resolve(output,'fixture-set.json'),fixtureBytes,{flag:'wx',mode:0o600}),
    writeFile(resolve(output,'guest-stdout.bin'),stdoutBytes,{flag:'wx',mode:0o600}),
  ]);
  return {case_count:receipt.executed,case_set_sha256:receipt.case_sha256};
}

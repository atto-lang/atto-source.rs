// Executable *native compiler CLI* gate for the canonical 43-case contract.
// Only the 38 target_parser cases can be checked here; typed loans, actor
// guest codegen, and cross-backend parity remain unexecuted until implemented.
// Source manifest is SHA-identical to atto-platform-conformance #545.
import test from 'node:test';
import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { readFileSync, mkdtempSync, existsSync, writeFileSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { spawnSync } from 'node:child_process';

const dir = resolve(fileURLToPath(new URL('.', import.meta.url)));
const contractPath = join(dir, 'fixtures/actor-projected-state-receivers-v1.json');
const bytes = readFileSync(contractPath);
const sha = createHash('sha1')
  .update('blob ' + bytes.length + '\0').update(bytes).digest('hex');
assert.equal(sha, '3b5d6f0a3d4698833073e63ee9af8bdad3bbbeb7',
  'canonical conformance contract changed: refresh proof and stage expectations');
const contract = JSON.parse(bytes.toString('utf8'));
assert.equal(contract.cases.length, 43);
const targets = contract.cases.filter(x => x.stage === 'target_parser');
assert.equal(targets.length, 38);
assert.equal(targets.filter(x => x.expect === 'accept_ast_only').length, 10);
assert.equal(targets.filter(x => x.expect === 'reject').length, 28);
assert.equal(new Set(contract.cases.map(x => x.id)).size, 43);
assert.ok(contract.cases.filter(x => x.stage !== 'target_parser')
  .every(x => x.evidence === 'NOT_EXECUTED'),
  'unimplemented typed/guest cases must remain explicitly unexecuted');

// Use a local build of the real Rust CLI, never a JS evaluator or a stub.
const compiler = resolve(dir, '../target/debug/attoc');

test('native attoc enforces all 38 parser-stage contract cases without guest artifacts', async t => {
  assert.ok(existsSync(compiler), 'build attoc before running this contract gate');
  for (const item of targets) {
    await t.test(item.id, () => {
      assert.deepEqual(Object.keys(item.sourceUnits), ['src/worker.atto']);
      assert.ok(item.expect === 'reject' || item.expect === 'accept_ast_only');
      const source = item.sourceUnits['src/worker.atto'];
      assert.ok(typeof source === 'string' && source.length > 0);
      const temp = mkdtempSync(join(tmpdir(), 'atto-projected-contract-'));
      try {
        const input = join(temp, 'worker.atto');
        const artifact = join(temp, 'result.wasm');
        writeFileSync(input, source, 'utf8');
        const run = spawnSync(compiler, [input, '-o', artifact], {
          cwd: temp, encoding: 'utf8', timeout: 12000, maxBuffer: 1 << 20
        });
        assert.ifError(run.error);
        assert.equal(run.status, 1, item.id + ': did not fail-closed; ' + run.stderr);
        assert.equal(run.stdout, '', item.id + ': compiler leaked artifact bytes on stdout');
        assert.equal(existsSync(artifact), false, item.id + ': rejected source emitted guest');
        const diag = run.stderr;
        // Diagnostic::Display formats "file:line:column: E_CODE: message".
        // Compare the *exact* leading diagnostic code, not a substring in
        // descriptive text (which could mask an unrelated failure).
        const match = diag.match(/:\s*(\d+):(\d+): (E_[A-Z0-9_]+):/);
        assert.ok(match, item.id + ': missing positioned diagnostic: ' + diag);
        assert.ok(Number(match[1]) >= 1 && Number(match[2]) >= 1,
          item.id + ': invalid diagnostic source position: ' + diag);
        const actualCode = match[3];
        if (item.expect === 'reject') {
          assert.match(item.diagnosticClass, /^E_[A-Z0-9_]+$/);
          assert.equal(actualCode, item.diagnosticClass,
            item.id + ': incorrect rejection category: ' + diag);
        } else {
          assert.ok(actualCode === 'E_UNSUPPORTED' || actualCode === 'E_UNSUPPORTED_TYPE',
            item.id + ': AST-only case failed too early or compiled unexpectedly: ' + diag);
        }
      } finally {
        rmSync(temp, { recursive: true, force: true });
      }
    });
  }
});

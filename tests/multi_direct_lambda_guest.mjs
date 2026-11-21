// Compile real .atto source with native attoc and execute generated guest Wasm.
// This is deliberately not an interpreter, JS equivalent or expected-only test.
import assert from 'node:assert/strict';
import { readFile, mkdtemp, rm, stat } from 'node:fs/promises';
import { spawnSync } from 'node:child_process';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';

const attoc = process.env.ATTOC
  || fileURLToPath(new URL('../target/debug/attoc', import.meta.url));
const root = new URL('./fixtures/', import.meta.url);
const cases = [
  { path: 'pass/lambda-direct-three-binds.atto', result: 42n },
  { path: 'pass/lambda-direct-sibling-captures.atto', result: 42n },
  { path: 'fail/lambda-direct-two-binds.atto', reject: 'E_UNSUPPORTED' },
];
const dir = await mkdtemp(join(tmpdir(), 'atto-multi-direct-'));
try {
  for (const [idx, row] of cases.entries()) {
    const source = fileURLToPath(new URL(row.path, root));
    const out = join(dir, idx + '.wasm');
    const cmd = spawnSync(attoc, [source, '-o', out], {
      encoding: 'utf8', timeout: 30_000, maxBuffer: 1 << 20,
    });
    assert.equal(cmd.error, undefined, row.path + ': native compiler process');
    if (row.reject) {
      assert.notEqual(cmd.status, 0, row.path + ': invalid guest was admitted');
      assert.ok(cmd.stderr.includes(row.reject), row.path + ': wrong diagnostic: ' + cmd.stderr);
      await assert.rejects(stat(out), { code: 'ENOENT' }, row.path + ': failed compile wrote guest');
      continue;
    }
    assert.equal(cmd.status, 0, row.path + ': ' + cmd.stderr);
    const bytes = await readFile(out);
    assert.ok(WebAssembly.validate(bytes), row.path + ': invalid Wasm bytes');
    const mod = await WebAssembly.compile(bytes);
    assert.deepEqual(WebAssembly.Module.imports(mod), [], row.path + ': unexpected host imports');
    const guest = await WebAssembly.instantiate(mod, {});
    assert.equal(guest.exports.main(), row.result, row.path + ': wrong guest return value');
    assert.equal(guest.exports.panic_code.value, 0, row.path + ': guest panic');
    console.log('PASS real multi-lambda guest', row.path, String(row.result));
  }
} finally {
  await rm(dir, { recursive: true, force: true });
}

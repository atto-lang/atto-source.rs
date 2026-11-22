// Actual compiler + Node WebAssembly guest execution, NOT a policy JS emulator.
// Run: cargo build --release --bin attoc
//      node --test tests/actor-stdlib-guest.test.mjs
import test from 'node:test';
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { mkdtempSync, readFileSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join, resolve, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const compiler = resolve(process.env.ATTOC || join(root, 'target/release/attoc'));

test('std.actor .atto imports execute as guest Wasm without ambient host imports', () => {
  const dir = mkdtempSync(join(tmpdir(), 'atto-std-actor-'));
  const output = join(dir, 'actor-policy.wasm');
  try {
    const build = spawnSync(compiler,
      ['stdlib/actor/guest_smoke.atto', '-o', output],
      { cwd: root, encoding: 'utf8', timeout: 60000 });
    assert.equal(build.error, undefined, String(build.error));
    assert.equal(build.status, 0, String(build.stderr));
    const wasm = readFileSync(output);
    assert.ok(WebAssembly.validate(wasm), 'native compiled Wasm required');
    const module = new WebAssembly.Module(wasm);
    assert.deepEqual(WebAssembly.Module.imports(module), [], 'policy cannot acquire host imports');
    const instance = new WebAssembly.Instance(module, {});
    assert.equal(typeof instance.exports.main, 'function');
    assert.equal(instance.exports.main(), 42n, 'all 34 actor-policy guest guards must hold');
    if (Object.hasOwn(instance.exports, 'panic_code')) {
      assert.equal(instance.exports.panic_code.value, 0, 'no guest trap');
    }
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

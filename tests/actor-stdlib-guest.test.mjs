// Real compiler + Node WebAssembly guest execution, not a JS policy emulator.
import test from 'node:test';
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { mkdtempSync, readFileSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join, resolve, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const compiler = resolve(process.env.ATTOC || join(root, 'target/release/attoc'));
for (const [fixture, label] of [
  ['stdlib/actor/guest_smoke.atto', '34 smoke assertions'],
  ['stdlib/actor/guest_edges.atto', '133 adversarial boundary assertions'],
]) {
  test('std.actor ' + label + ' executes as linked Atto Wasm with no host imports', () => {
    const dir = mkdtempSync(join(tmpdir(), 'atto-std-actor-'));
    const output = join(dir, 'actor-policy.wasm');
    try {
      const build = spawnSync(compiler, [fixture, '-o', output],
        { cwd: root, encoding: 'utf8', timeout: 60000, windowsHide: true });
      assert.equal(build.error, undefined, 'compiler could not start: ' + String(build.error));
      assert.equal(build.signal, null, 'compiler aborted: ' + String(build.stderr));
      assert.equal(build.status, 0, 'real Atto guest compilation failed: ' + String(build.stderr));
      const wasm = readFileSync(output);
      assert.ok(WebAssembly.validate(wasm), 'actual compiled Wasm required');
      const module = new WebAssembly.Module(wasm);
      assert.deepEqual(WebAssembly.Module.imports(module), [], 'policy must not acquire host imports');
      const instance = new WebAssembly.Instance(module, {});
      assert.equal(typeof instance.exports.main, 'function');
      assert.equal(instance.exports.main(), 42n, 'guest policy guard failure in ' + fixture);
      if (Object.hasOwn(instance.exports, 'panic_code')) {
        assert.equal(instance.exports.panic_code.value, 0, 'unexpected guest trap');
      }
    } finally {
      rmSync(dir, { recursive: true, force: true });
    }
  });
}

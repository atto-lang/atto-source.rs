// Real compiler -> Wasm -> guest execution, not just parser acceptance.
import assert from 'node:assert/strict';
import { mkdtemp, writeFile, readFile, rm, stat } from 'node:fs/promises';
import { spawnSync } from 'node:child_process';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
const attoc = process.env.ATTOC || 'target/debug/attoc';
const root = await mkdtemp(join(tmpdir(), 'atto-await-'));
const header = 'fnc identity(int n): int { return n; }\n';
const cases = [
  ['return 42 |> await;', 42n],
  ['return 42 |> await |> identity;', 42n],
  ['return await 42;', 42n],
  ['return identity(await 42);', 42n],
  ['return identity(41) + 1 |> await |> identity;', 42n],
  ['return 42 |> await |> await;', 42n],
  ['return true |> await;', null],
  ['return 42 |> await(7);', null],
  ['return 42 |> await<int>;', null],
  ['return await true;', null],
  ['return 42 |> std.await;', null],
];
try {
  for (const [i, [body, expected]] of cases.entries()) {
    const src = join(root, i + '.atto'), bin = join(root, i + '.wasm');
    await writeFile(src, header + 'pub fnc main(): int { ' + body + ' }\n');
    const r = spawnSync(attoc, [src, '-o', bin], {encoding:'utf8', timeout:30000});
    assert.ifError(r.error);
    if (expected === null) {
      assert.notEqual(r.status, 0, body);
      await assert.rejects(stat(bin), {code:'ENOENT'});
      continue;
    }
    assert.equal(r.status, 0, body + ': ' + r.stderr);
    const bytes = await readFile(bin);
    assert.ok(WebAssembly.validate(bytes), body);
    const {instance} = await WebAssembly.instantiate(bytes, {atto:{}});
    assert.equal(instance.exports.main(), expected, body);
    assert.equal(instance.exports.panic_code.value, 0, body);
  }
  console.log('PASS ' + cases.length + ' immediate await guest and rejection cases');
} finally { await rm(root, {recursive:true,force:true}); }

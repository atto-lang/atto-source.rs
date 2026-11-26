// Real compiler -> Wasm -> guest execution, not just parser acceptance.
import assert from 'node:assert/strict';
import { mkdtemp, writeFile, readFile, rm, stat } from 'node:fs/promises';
import { spawnSync } from 'node:child_process';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
const attoc = process.env.ATTOC || 'target/debug/attoc';
const root = await mkdtemp(join(tmpdir(), 'atto-directional-'));
const header = 'fnc identity(int n): int { return n; }\n';
const cases = [
  ['42 >> val int n; return n;', 42n],
  ['val int n << 42; return n;', 42n],
  ['41 |> identity |> await >> val n; return n+1;', 42n],
  ['val int n << 41 |> identity; return n+1;', 42n],
  ['42 >> const int n; return n;', 42n],
  ['const int n << 21 <<< 1; return n;', 42n],
  ['84 >>> 1 >> val int n; return n;', 42n],
  ['let int n << 41; n=42; return n;', 42n],
  ['41 >> let int n; n=42; return n;', 42n],
  ['42 >> val int n; n=0; return n;', null],
  ['true >> val int n; return n;', null],
  ['val bool n << 42; return 0;', null],
  ['42 >> n; return 0;', null],
  ['42 > > val n; return n;', null],
  ['42 >> val int n; 0 >> val int n; return n;', null],
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
  console.log('PASS ' + cases.length + ' directional declaration guest and rejection cases');
} finally { await rm(root, {recursive:true,force:true}); }

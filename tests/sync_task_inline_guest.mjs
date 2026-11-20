// Execute true native-compiled sync task guests, not parser-only checks.
import assert from 'node:assert/strict';
import { mkdtemp, writeFile, readFile, rm, stat } from 'node:fs/promises';
import { spawnSync } from 'node:child_process';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';

const attoc = process.env.ATTOC || fileURLToPath(new URL('../target/debug/attoc', import.meta.url));
const pass = [
  ['do sync task {}; return 42;', 42n],
  ['do sync task<void> {}; return 42;', 42n],
  ['do sync task { return; }; return 42;', 42n],
  ['do sync task<void> { return; }; return 42;', 42n],
  ['val x = sync task<int> { return 40 + 2; }; return x;', 42n],
  ['val x = 40; return sync task<int> { return x + 2; };', 42n],
  ['do sync task {}; val result = sync task<int> { return 40 + 2; }; do sync task {}; return result;', 42n],
  ['val x = sync task<bool> { return true; }; return 42;', 42n],
];
const fail = [
  ['sync task {}; return 42;', 'E_SYNTAX'],
  ['sync task<void> {}; return 42;', 'E_SYNTAX'],
  ['sync task<int> { return 42; }; return 42;', 'E_SYNTAX'],
  ['do sync task<int> {}; return 42;', 'E_TYPE'],
  ['do sync task<bool> {}; return 42;', 'E_TYPE'],
  ['do sync task<int> { return 7; }; return 42;', 'E_TYPE'],
  ['do sync task { return 7; }; return 42;', 'E_TYPE'],
  ['do sync task<void> { return 7; }; return 42;', 'E_TYPE'],
  ['val x = sync task<int> { return true; }; return 42;', 'E_TYPE'],
  ['val x = sync task<int> { val y = 42; return y; }; return x;', 'E_UNSUPPORTED'],
  ['val x = sync task<int> { return stdio.println(42); }; return x;', 'E_UNSUPPORTED'],
  ['val x = sync task<int> { return await task<int> { return 42; }; }; return x;', 'E_UNSUPPORTED'],
  ['val x = task<int> { return 42; }; return 0;', 'E_UNSUPPORTED'],
];
const work = await mkdtemp(join(tmpdir(), 'atto-sync-task-'));
try {
  let i = 0;
  for (const [body, expected] of pass) {
    const input = join(work, `pass-${i}.atto`);
    const out = join(work, `pass-${i++}.wasm`);
    await writeFile(input, `pub fnc main(): int { ${body} }\n`);
    const res = spawnSync(attoc, [input, '-o', out], {encoding: 'utf8', timeout: 30000});
    assert.equal(res.status, 0, res.stderr);
    const bytes = await readFile(out);
    assert.ok(WebAssembly.validate(bytes), body);
    const module = new WebAssembly.Module(bytes);
    const guest = new WebAssembly.Instance(module, {atto: {}});
    assert.equal(guest.exports.main(), expected, body);
    assert.equal(guest.exports.panic_code.value, 0, body);
  }
  i = 0;
  for (const [body, code] of fail) {
    const input = join(work, `fail-${i}.atto`);
    const out = join(work, `fail-${i++}.wasm`);
    await writeFile(input, `pub fnc main(): int { ${body} }\n`);
    const res = spawnSync(attoc, [input, '-o', out], {encoding: 'utf8', timeout: 30000});
    assert.equal(res.status, 1, body + ': expected rejection; ' + res.stderr);
    assert.ok(res.stderr.includes(code), body + ': missing ' + code + ': ' + res.stderr);
    await assert.rejects(stat(out), {code: 'ENOENT'});
  }
  console.log('PASS', pass.length, 'executed guest sync-task fixtures and', fail.length, 'no-artifact rejections');
} finally {
  await rm(work, {recursive: true, force: true});
}

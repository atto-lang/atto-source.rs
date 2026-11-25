// Actual native attoc -> Wasm guest execution; not a parser-only receipt.
import assert from 'node:assert/strict';
import { mkdtemp, writeFile, readFile, rm, access } from 'node:fs/promises';
import { spawnSync } from 'node:child_process';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';

const attoc = process.env.ATTOC || fileURLToPath(new URL('../target/debug/attoc', import.meta.url));
const main = body => `pub fnc main(): int { ${body} }\n`;
const cases = [
  [main('return 2 + 3 * 4;'), 14n],
  [main('return (2 + 3) * 4;'), 20n],
  [main('return 20 - 5 - 3;'), 12n],
  [main('return 20 - (5 - 3);'), 18n],
  [main('return -((2 + 3) * 4);'), -20n],
  [main('return (((42)));'), 42n],
  [main('return (2 + /* ), */ 5) * 6;'), 42n],
  [main('return ((true || false) && true) ? 42 : 0;'), 42n],
  [main('return (false && (true || true)) ? 0 : 42;'), 42n],
  [main('let (a, b, c) = (5, 6, true); return c ? a * b + 12 : 0;'), 42n],
  ['fnc add(int a, int b): int { return a + b; }\n' + main('return add((5 + 6), (30 + 1));'), 42n],
];
const malformed = [
  'return (5,,6);', 'return (5 + 6];', 'return (5 + );',
  'return (a=5,6);', 'return (5,a=6);', 'return (5,6.true);',
  'return (5 + (6);',
];
const dir = await mkdtemp(join(tmpdir(), 'atto-grouping-'));
try {
  let n = 0;
  for (const [source, expected] of cases) {
    const src = join(dir, `${n}.atto`), out = join(dir, `${n++}.wasm`);
    await writeFile(src, source);
    const result = spawnSync(attoc, [src, '-o', out], { encoding: 'utf8', timeout: 30000 });
    assert.ifError(result.error);
    assert.equal(result.status, 0, `${source}: ${result.stderr}`);
    const bytes = await readFile(out);
    assert.ok(WebAssembly.validate(bytes));
    const { instance } = await WebAssembly.instantiate(bytes, { atto: {} });
    assert.equal(instance.exports.main(), expected, source);
    assert.equal(instance.exports.panic_code.value, 0, source);
  }
  for (const body of malformed) {
    const src = join(dir, `${n}.atto`), out = join(dir, `${n++}.wasm`);
    await writeFile(src, main(body));
    const result = spawnSync(attoc, [src, '-o', out], { encoding: 'utf8', timeout: 30000 });
    assert.ifError(result.error);
    assert.ok(result.status !== null && result.status !== 0 && result.signal === null, body);
    await assert.rejects(access(out), { code: 'ENOENT' }, body);
  }
  console.log('PASS', cases.length, 'executed grouping/tuple guests;', malformed.length, 'no-artifact negatives');
} finally {
  await rm(dir, { recursive: true, force: true });
}

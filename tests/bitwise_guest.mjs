// Execute native-compiled AttoLang bitwise guests; not merely parser acceptance.
import assert from 'node:assert/strict';
import { mkdtemp, writeFile, readFile, rm } from 'node:fs/promises';
import { spawnSync } from 'node:child_process';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';

const attoc = process.env.ATTOC || fileURLToPath(new URL('../target/debug/attoc', import.meta.url));
const cases = [
  ['12 & 10', 8n], ['12 | 10', 14n], ['12 ^^ 10', 6n],
  ['~12', -13n], ['8 <<< 2', 32n], ['32 >>> 2', 8n],
  ['-8 ashr 1', -4n], ['-1 >>> 1', 9223372036854775807n],
  ['1 <<< 63', -9223372036854775808n], ['1 + 2 <<< 3', 24n], ['64 >>> 1 + 1', 16n],
  ['12 & 10 | 2 ^^ 6', 12n],
  ['(1 | 2) <<< 1', 6n],
  ['(1 <<< 63) >>> 63', 1n],
  ['(1 <<< 63) ashr 63', -1n],
  ['(1 <<< 63) <<< 1', 0n],
  ['(1 ^^ 2) & 3', 3n],
  ['0b101100 & 0b001100', 12n],
  ['0B1011 >>> 1', 5n],
];
const compounds = [
  ['x &= 10', 8n], ['x |= 2', 14n], ['x ^^= 10', 6n],
  ['x <<<= 2', 48n], ['x >>>= 1', 6n], ['x ashr= 1', 6n],
];
const tmp = await mkdtemp(join(tmpdir(), 'atto-bitwise-'));
try {
  let i = 0;
  for (const [expression, expected] of [
    ...cases.map(([e, n]) => [`return ${e};`, n]),
    ...compounds.map(([e, n]) => [`let x: int = 12; ${e}; return x;`, n]),
  ]) {
    const src = join(tmp, `${i}.atto`);
    const bin = join(tmp, `${i}.wasm`);
    i++;
    await writeFile(src, `pub fnc main(): int { ${expression} }\n`);
    const result = spawnSync(attoc, [src, '-o', bin], { encoding: 'utf8', timeout: 30000 });
    assert.equal(result.status, 0, `${expression}: ${result.stderr}`);
    const bytes = await readFile(bin);
    assert.ok(WebAssembly.validate(bytes), `${expression}: invalid wasm`);
    const { instance } = await WebAssembly.instantiate(bytes, { atto: {} });
    assert.equal(instance.exports.main(), expected, expression);
    assert.equal(instance.exports.panic_code.value, 0, expression);
  }
  // Source programs using the retired spelling must fail compilation
  // without leaving even an empty guest artifact.
  for (const [index, body] of [
    'return 12 ^ 10;',
    'return 12 ^ ^ 10;',
    'let x: int = 12; x ^= 3; return x;',
    'let x: int = 12; x ^ ^= 3; return x;',
    'let x: int = 12; x ^^ = 3; return x;',
    'let x: int = 12; x ^/*gap*/^= 3; return x;',
    'let x: int = 12; x ^^/*gap*/= 3; return x;',
    'val x: int = 12; x ^^= 3; return x;',
    'let x: int = 12; x ^^= true; return x;',
    'return 1 <<< 64;',
    'return 1 >>> -1;',
  ].entries()) {
    const src = join(tmp, `reject-${index}.atto`);
    const bin = join(tmp, `reject-${index}.wasm`);
    await writeFile(src, `pub fnc main(): int { ${body} }\n`);
    const result = spawnSync(attoc, [src, '-o', bin],
      { encoding: 'utf8', timeout: 30000 });
    assert.notEqual(result.status, 0, 'deprecated caret syntax compiled unexpectedly');
    await assert.rejects(readFile(bin), { code: 'ENOENT' },
      'rejected syntax may not leave any guest artifact');
  }
  // Dynamic counts are evaluated exactly once, checked before reaching
  // Wasm's naturally modulo-64 shift instructions, and raise panic code 7.
  for (const [index, operation] of ['1 <<< distance', '1 >>> distance',
      '-8 ashr distance'].entries()) {
    const src = join(tmp, `dynamic-${index}.atto`);
    const bin = join(tmp, `dynamic-${index}.wasm`);
    await writeFile(src,
      `pub fnc main(): int { let distance: int = 64; return ${operation}; }\n`);
    const result = spawnSync(attoc, [src, '-o', bin],
      { encoding: 'utf8', timeout: 30000 });
    assert.equal(result.status, 0, result.stderr);
    const bytes = await readFile(bin);
    const { instance } = await WebAssembly.instantiate(bytes, { atto: {} });
    assert.throws(() => instance.exports.main(), WebAssembly.RuntimeError);
    assert.equal(instance.exports.panic_code.value, 7, operation);
  }
  console.log('PASS', i, 'bitwise Wasm guests; 11 no-artifact and 3 runtime shift panic checks');
} finally {
  await rm(tmp, { recursive: true, force: true });
}

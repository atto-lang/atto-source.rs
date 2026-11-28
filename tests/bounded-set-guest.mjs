import assert from 'node:assert/strict';
import { mkdtempSync, writeFileSync, readFileSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { pathToFileURL } from 'node:url';
import { spawnSync } from 'node:child_process';
const dir = mkdtempSync(join(tmpdir(), 'atto-set-'));
const compiler = resolve(process.env.ATTO_COMPILER_BIN ?? 'target/debug/attoc');
try {
  const cases = [
    ['fnc main(): int { let s = Set.of(7, 7, 42); return s.len(); }', 2n],
    ['fnc main(): int { let n = -7; let s = Set.of(n, n + 49, n); return s.contains(42) ? 42 : 0; }', 42n],
    ['fnc main(): int { let s = Set.of(-9223372036854775808, 9223372036854775807, -9223372036854775808); return s.len(); }', 2n],
    ['fnc main(): int { let s = Set.of(0, -1); return s.contains(42) ? 1 : 42; }', 42n],
    ['fnc main(): int { let s = Set.of(1,2,3,4,5,6,7,8); return s.len(); }', 8n],
  ];
  for (let i = 0; i < cases.length; i++) {
    const [src, expected] = cases[i];
    const input = join(dir, i + '.atto');
    writeFileSync(input, src);
    for (const mode of ['wasm', 'js']) {
      const output = join(dir, i + (mode === 'wasm' ? '.wasm' : '.mjs'));
      const args = mode === 'wasm' ? [input, '-o', output] : ['--emit', 'js', input, '-o', output];
      const run = spawnSync(compiler, args, { encoding: 'utf8' });
      assert.equal(run.status, 0, run.stderr);
      const guest = mode === 'wasm'
        ? (await WebAssembly.instantiate(readFileSync(output), {})).instance
        : (await import(pathToFileURL(output).href)).instantiate();
      for (let j = 0; j < 2; j++) {
        assert.equal(guest.exports.main(), expected);
        assert.equal(guest.exports.panic_code.value, 0);
      }
      console.log('SET_GUEST_EXECUTED', mode, i, String(expected));
    }
  }
} finally { rmSync(dir, {recursive:true, force:true}); }

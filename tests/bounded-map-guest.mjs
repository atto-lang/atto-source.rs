import assert from 'node:assert/strict';
import { mkdtempSync, writeFileSync, readFileSync, rmSync, existsSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { pathToFileURL } from 'node:url';
import { spawnSync } from 'node:child_process';
const dir = mkdtempSync(join(tmpdir(), 'atto-map-'));
const compiler = resolve(process.env.ATTO_COMPILER_BIN ?? 'target/debug/attoc');
try {
  const cases = [
    ['fnc main(): int { let n = 40; let m = Map.of(Entry.of(-7,n+2),Entry.of(3,9)); return m.get_or(-7,0); }',42n],
    ['fnc main(): int { let m = Map.of(Entry.of(1,2)); let n = m.len(); return m.contains_key(1) ? n : 0; }',1n],
    ['fnc main(): int { let m = Map.of(Entry.of(-9223372036854775808,42),Entry.of(9223372036854775807,9)); return m.get_or(-9223372036854775808,0); }',42n],
    ['fnc main(): int { let m = Map.of(Entry.of(1,2)); return m.get_or(9,42); }',42n],
    ['fnc main(): int { let m = Map.of(Entry.of(1,2),Entry.of(2,3),Entry.of(3,4),Entry.of(4,5),Entry.of(5,6),Entry.of(6,7),Entry.of(7,8),Entry.of(8,9)); return m.len(); }',8n],
  ];
  for (let i = 0; i < cases.length; i++) {
    const [src, expected] = cases[i];
    const input = join(dir, i + '.atto');
    writeFileSync(input, src);
    for (const mode of ['wasm', 'js']) {
      const output = join(dir, i + (mode === 'wasm' ? '.wasm' : '.mjs'));
      const args = mode === 'wasm' ? [input, '-o', output] : ['--emit=js', input, '-o', output];
      const run = spawnSync(compiler, args, { encoding: 'utf8' });
      assert.ifError(run.error);
      assert.equal(run.signal, null);
      assert.equal(run.status, 0, run.stderr);
      const guest = mode === 'wasm'
        ? (await WebAssembly.instantiate(readFileSync(output), {})).instance
        : (await import(pathToFileURL(output).href)).instantiate();
      for (let j = 0; j < 2; j++) {
        assert.equal(guest.exports.main(), expected);
        assert.equal(guest.exports.panic_code.value, 0);
      }
      console.log('MAP_GUEST_EXECUTED', mode, i, String(expected));
    }
  }
  for (const [i, src] of [
    'fnc main(): int { let m = Map.of(Entry.of(0,1),Entry.of(-0,2)); return m.len(); }',
    'fnc main(): int { let k = 1; let m = Map.of(Entry.of(k,2)); return m.len(); }',
    'fnc main(): int { let m = Map.of(Entry.of(1,true)); return m.len(); }',
    'fnc main(): int { let m = Map.of(Entry.of(1,2)); let a = m; return a.len(); }',
    'fnc main(): int { let m = Map.of(Entry.of(1,2)); m.insert(3,4); return m.len(); }',
  ].entries()) {
    const input = join(dir, 'bad-' + i + '.atto');
    writeFileSync(input, src);
    for (const mode of ['wasm', 'js']) {
      const output = join(dir, 'bad-' + i + (mode === 'wasm' ? '.wasm' : '.mjs'));
      const args = mode === 'wasm' ? [input, '-o', output] : ['--emit=js', input, '-o', output];
      const run = spawnSync(compiler, args, {encoding:'utf8'});
      assert.ifError(run.error);
      assert.equal(run.signal, null);
      assert.notEqual(run.status, 0, src);
      assert.equal(existsSync(output), false, src);
      console.log('MAP_REJECT_NO_ARTIFACT', mode, i);
    }
  }
} finally { rmSync(dir, {recursive:true, force:true}); }

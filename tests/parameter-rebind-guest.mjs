// Execute actual compiled guests; no mock compiler or success on skip.
import assert from 'node:assert/strict';
import { mkdtempSync, writeFileSync, readFileSync, rmSync, existsSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { spawnSync } from 'node:child_process';
const compiler = resolve(process.env.ATTOC || 'target/release/attoc');
const dir = mkdtempSync(join(tmpdir(), 'atto-rebind-'));
function build(source, explicit = false) {
  const input = join(dir, 'main.atto');
  const output = join(dir, 'main.wasm');
  writeFileSync(input, source);
  rmSync(output, { force: true });
  const r = spawnSync(compiler, [...(explicit ? ['--explicit-rebinding'] : []), 'main.atto', '-o', output], {
    cwd: dir, encoding: 'utf8', timeout: 30000
  });
  assert.ifError(r.error);
  return { r, output };
}
function guest(source, expected, explicit = false) {
  const { r, output } = build(source, explicit);
  assert.equal(r.status, 0, r.stderr);
  const bytes = readFileSync(output);
  assert.ok(WebAssembly.validate(bytes));
  const module = new WebAssembly.Module(bytes);
  assert.deepEqual(WebAssembly.Module.imports(module), []);
  const instance = new WebAssembly.Instance(module, {});
  assert.equal(instance.exports.main(), BigInt(expected));
  assert.equal(instance.exports.panic_code.value, 0);
  return bytes;
}
function reject(source, code, explicit = false) {
  const { r, output } = build(source, explicit);
  assert.equal(r.status, 1, r.stderr);
  assert.match(r.stderr, new RegExp(code));
  assert.equal(existsSync(output), false);
}
try {
  // Caller retains its original scalar; only the local formal changes.
  const main = 'pub fnc main(): int { val int n = 20; return bump(n) + n; }';
  guest('fnc bump(rebind int n): int { n = n + 2; return n; } ' + main, 42);
  guest('fnc bump(rebind n: int): int { n = n + 2; return n; } ' + main, 42);
  reject('fnc bump(int n): int { n = n + 2; return n; } ' + main, 'E_IMMUTABLE');
  const fixed = 'fnc id(int n): int { return n; } pub fnc main(): int { return id(42); }';
  assert.deepEqual(guest(fixed, 42), guest(fixed.replace('int n', 'rebind int n'), 42));
  reject('fnc replace(rebind mut Foo f): void {} pub fnc main(): int { return 0; }', 'E_UNSUPPORTED');
  guest('pub fnc main(): int { let int x = 39; set x = x + 1; set! x = x + 1; set! (x = x + 1); return x; }', 42);
  guest('pub fnc main(): int { let x = 40; let x = x + 1; set x = x + 1; return x; }', 42);
  reject('pub fnc main(): int { const x = 40; set! x = 42; return x; }', 'E_IMMUTABLE');
  reject('pub fnc main(): int { const x = 40; const x = 42; return x; }', 'E_DUPLICATE');
  guest('pub fnc main(): int { let x = 0; for (set x = 40; x < 42; set! (x = x + 1)) {} return x; }', 42, true);
  guest('pub fnc main(): int { let x = 40; set x = x + 2; return x; }', 42, true);
  reject('pub fnc main(): int { let x = 40; x = 42; return x; }', 'E_EXPLICIT_REBIND', true);
  reject('pub fnc main(): int { let x = 0; for (x = 40; x < 42; x++) {} return x; }', 'E_EXPLICIT_REBIND', true);
  reject('pub fnc main(): int { let x = 40; for (; x < 42; x = x + 1) {} return x; }', 'E_EXPLICIT_REBIND', true);
  console.log('scalar rebind guest cases passed (including explicit binding preview)');
} finally {
  rmSync(dir, { recursive: true, force: true });
}

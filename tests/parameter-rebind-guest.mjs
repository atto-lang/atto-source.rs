// Execute actual compiled guests; no mock compiler or success on skip.
import assert from 'node:assert/strict';
import { mkdtempSync, writeFileSync, readFileSync, rmSync, existsSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { spawnSync } from 'node:child_process';
const compiler = resolve(process.env.ATTOC || 'target/release/attoc');
const dir = mkdtempSync(join(tmpdir(), 'atto-rebind-'));
function build(source) {
  const input = join(dir, 'main.atto');
  const output = join(dir, 'main.wasm');
  writeFileSync(input, source);
  rmSync(output, { force: true });
  const r = spawnSync(compiler, ['main.atto', '-o', output], {
    cwd: dir, encoding: 'utf8', timeout: 30000
  });
  assert.ifError(r.error);
  return { r, output };
}
function guest(source, expected) {
  const { r, output } = build(source);
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
function reject(source, code) {
  const { r, output } = build(source);
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
  console.log('scalar rebind guest cases passed');
} finally {
  rmSync(dir, { recursive: true, force: true });
}

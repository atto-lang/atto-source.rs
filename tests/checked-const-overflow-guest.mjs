// Executable complement to checked_const_overflow_boundary.rs.
// The scalar-v4 corpus in PR #399 covers broad arithmetic values; this tests
// specifically whether const classification changes runtime trap behavior.
import test from 'node:test';
import assert from 'node:assert/strict';
import {spawnSync} from 'node:child_process';
import {mkdtempSync, readFileSync, writeFileSync, existsSync, rmSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {join} from 'node:path';

const attoc = process.env.ATTOC;
assert.ok(attoc, 'ATTOC must point to a built native Rust compiler; missing is not a skip');

function compile(source) {
  const dir = mkdtempSync(join(tmpdir(), 'atto-const-overflow-'));
  try {
    const input = join(dir, 'main.atto'), output = join(dir, 'main.wasm');
    writeFileSync(input, source, {flag: 'wx'});
    const result = spawnSync(attoc, [input, '-o', output], {
      encoding: 'utf8', timeout: 15000, maxBuffer: 1 << 20,
    });
    assert.equal(result.error, undefined, 'compiler must start and finish');
    assert.equal(result.signal, null, 'compiler must not crash');
    return {
      status: result.status,
      stderr: result.stderr,
      stdout: result.stdout,
      published: existsSync(output),
      wasm: existsSync(output) ? readFileSync(output) : null,
    };
  } finally {
    rmSync(dir, {recursive: true, force: true});
  }
}

function runGuest(wasm) {
  assert.ok(wasm && WebAssembly.validate(wasm), 'actual checked guest Wasm required');
  const module = new WebAssembly.Module(wasm);
  assert.deepEqual(WebAssembly.Module.imports(module), [], 'no unexpected guest host capability');
  const instance = new WebAssembly.Instance(module, {});
  return instance.exports;
}

test('local constant overflow fails before Wasm output', () => {
  const compiled = compile('pub fnc main(): int { const value = 9223372036854775807 + 1; return value; }');
  assert.equal(compiled.status, 1);
  assert.equal(compiled.published, false, 'rejection published a guest artifact');
  assert.match(compiled.stderr, /E_CONST_OVERFLOW/);
});

test('ordinary return overflow compiles but traps with canonical panic_code=1', () => {
  const compiled = compile('pub fnc main(): int { return 9223372036854775807 + 1; }');
  assert.equal(compiled.status, 0, compiled.stderr);
  const guest = runGuest(compiled.wasm);
  assert.throws(() => guest.main(), WebAssembly.RuntimeError);
  assert.equal(guest.panic_code.value, 1);
});

test('unused ordinary val initializer still traps before subsequent return', () => {
  const compiled = compile('pub fnc main(): int { val unused = 3037000500 * 3037000500; return 42; }');
  assert.equal(compiled.status, 0, compiled.stderr);
  const guest = runGuest(compiled.wasm);
  assert.throws(() => guest.main(), WebAssembly.RuntimeError);
  assert.equal(guest.panic_code.value, 1);
});

test('an unselected overflowing const ternary arm stays lazy', () => {
  const compiled = compile('pub fnc main(): int { const result = false ? 9223372036854775807 + 1 : 42; return result; }');
  assert.equal(compiled.status, 0, compiled.stderr);
  const guest = runGuest(compiled.wasm);
  assert.equal(guest.main(), 42n);
  assert.equal(guest.panic_code.value, 0);
});

test('dynamic interprocedural overflow stays a runtime trap', () => {
  const compiled = compile('fnc add(a: int, b: int): int { return a + b; } pub fnc main(): int { return add(9223372036854775807, 1); }');
  assert.equal(compiled.status, 0, compiled.stderr);
  const guest = runGuest(compiled.wasm);
  assert.throws(() => guest.main(), WebAssembly.RuntimeError);
  assert.equal(guest.panic_code.value, 1);
});

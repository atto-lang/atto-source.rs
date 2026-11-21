import test from 'node:test';
import assert from 'node:assert/strict';
import {readCompilerDiagnostic} from '../scripts/compiler-diagnostic.mjs';

function memory() { return new WebAssembly.Memory({initial: 1, maximum: 1}); }

test('reads a real ABI C string and permits a 511-byte payload', () => {
  const mem = memory(), bytes = new Uint8Array(mem.buffer);
  bytes.set(new TextEncoder().encode('E_UNBOUND: unknown variable'), 11);
  assert.equal(readCompilerDiagnostic(mem, 11), 'E_UNBOUND: unknown variable');
  bytes.fill(65, 48, 48 + 511);
  bytes[48 + 511] = 0;
  assert.equal(readCompilerDiagnostic(mem, 48), 'A'.repeat(511));
});

test('rejects negative, fractional, NaN, infinite or out-of-range pointers', () => {
  const mem = memory();
  for (const bad of [-1, 1.5, NaN, Infinity, -Infinity, 65536, Number.MAX_SAFE_INTEGER + 1]) {
    assert.throws(() => readCompilerDiagnostic(mem, bad), /E_COMPILER_ABI/);
  }
  assert.throws(() => readCompilerDiagnostic({}, 0), /E_COMPILER_ABI/);
});

test('rejects unterminated diagnostics at the memory boundary and at 512 bytes', () => {
  const mem = memory(), bytes = new Uint8Array(mem.buffer);
  bytes.fill(88, bytes.length - 8);
  assert.throws(() => readCompilerDiagnostic(mem, bytes.length - 8), /unterminated diagnostic/);
  bytes.fill(89, 256, 256 + 512);
  bytes[256 + 512] = 0;
  assert.throws(() => readCompilerDiagnostic(mem, 256), /unterminated diagnostic/);
});

test('rejects invalid UTF-8 instead of silently substituting U+FFFD', () => {
  const mem = memory(), bytes = new Uint8Array(mem.buffer);
  bytes.set([0xc3, 0x28, 0x00], 13);
  assert.throws(() => readCompilerDiagnostic(mem, 13), TypeError);
});

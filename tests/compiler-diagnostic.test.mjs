import test from 'node:test';
import assert from 'node:assert/strict';
import {readCompilerDiagnostic, verifyCompilerDiagnosticParity} from '../scripts/compiler-diagnostic.mjs';

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


test('native and wasm32 diagnostics must agree on full bytes and source position', () => {
  const label='/tmp/path with spaces/a:b.atto';
  const diagnostic={ok:false,line:2,col:4,message:'E_TOKEN: unexpected bullet lookalike'};
  const stderr=label+': 2:4: E_TOKEN: unexpected bullet lookalike\n';
  assert.deepEqual(verifyCompilerDiagnosticParity(stderr,label,diagnostic,'E_TOKEN'),
    {code:'E_TOKEN',line:2,col:4,message:diagnostic.message});
  assert.throws(()=>verifyCompilerDiagnosticParity(
    label+': 2:5: E_TOKEN: unexpected bullet lookalike\n',label,diagnostic,'E_TOKEN'),
    /not byte-identical/);
  assert.throws(()=>verifyCompilerDiagnosticParity(
    label+': 2:4: E_TOKEN: different message\n',label,diagnostic,'E_TOKEN'),
    /not byte-identical/);
  assert.throws(()=>verifyCompilerDiagnosticParity(stderr+'unexpected warning\n',
    label,diagnostic,'E_TOKEN'), /not byte-identical/);
});

test('native and wasm32 diagnostics reject wrong codes and malformed ABI fields', () => {
  const label='source.atto';
  const diagnostic={ok:false,line:1,col:1,message:'E_ARITY: expected 2 arguments'};
  const stderr=label+': 1:1: '+diagnostic.message+'\n';
  assert.throws(()=>verifyCompilerDiagnosticParity(stderr,label,diagnostic,'E_TOKEN'),
    /different diagnostic code/);
  assert.throws(()=>verifyCompilerDiagnosticParity(stderr,label,{...diagnostic,line:1.5},
    'E_ARITY'),/invalid diagnostic line/);
  assert.throws(()=>verifyCompilerDiagnosticParity(stderr,label,{...diagnostic,col:-1},
    'E_ARITY'),/invalid diagnostic column/);
  assert.throws(()=>verifyCompilerDiagnosticParity(stderr,label,{...diagnostic,ok:true},
    'E_ARITY'),/explicit diagnostic result/);
  assert.throws(()=>verifyCompilerDiagnosticParity(stderr,label,{...diagnostic,
    message:'E_ARITY: bad\\nline'},'E_ARITY'),/invalid diagnostic format/);
  assert.throws(()=>verifyCompilerDiagnosticParity(stderr,'bad\nlabel',diagnostic,
    'E_ARITY'),/invalid native source label/);
});

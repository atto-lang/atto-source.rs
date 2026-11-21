// Strict, bounded diagnostic decoding for the real Rust/Wasm compiler ABI.
// Missing NUL terminators, malformed UTF-8 and non-Wasm memory fail closed.
import assert from 'node:assert/strict';

const decoder = new TextDecoder('utf-8', {fatal: true});
const MAX_DIAGNOSTIC_BYTES = 512;

export function readCompilerDiagnostic(memory, address) {
  assert.ok(memory instanceof WebAssembly.Memory,
    'E_COMPILER_ABI: compiler did not export linear memory');
  assert.ok(Number.isSafeInteger(address) && address >= 0,
    'E_COMPILER_ABI: invalid diagnostic pointer');
  const bytes = new Uint8Array(memory.buffer);
  assert.ok(address < bytes.length,
    'E_COMPILER_ABI: diagnostic pointer outside linear memory');
  const limit = Math.min(bytes.length, address + MAX_DIAGNOSTIC_BYTES);
  let end = address;
  while (end < limit && bytes[end] !== 0) end++;
  assert.ok(end < limit && bytes[end] === 0,
    'E_COMPILER_ABI: unterminated diagnostic');
  return decoder.decode(bytes.subarray(address, end));
}


// Reject diagnostic drift between native attoc and the wasm32-hosted Rust
// compiler. Both invoke the same Diagnostic::render/Display implementation;
// accepting a mere substring such as E_TOKEN could mask changed source
// positions, incorrect error prose, or unexpected extra stderr output.
export function verifyCompilerDiagnosticParity(stderr, sourceLabel, wasmDiagnostic, expectedCode) {
  assert.match(expectedCode, /^E_[A-Z0-9_]+$/, 'invalid expected diagnostic code');
  assert.ok(typeof sourceLabel === 'string' && sourceLabel.length > 0 &&
    !/[\r\n\0]/.test(sourceLabel), 'invalid native source label');
  assert.ok(wasmDiagnostic && wasmDiagnostic.ok === false,
    'wasm compiler must return an explicit diagnostic result');
  for (const [name, value] of [['line', wasmDiagnostic.line], ['column', wasmDiagnostic.col]]) {
    assert.ok(Number.isSafeInteger(value) && value >= 0,
      'E_COMPILER_ABI: invalid diagnostic ' + name);
  }
  const message = wasmDiagnostic.message;
  assert.ok(typeof message === 'string' &&
    /^E_[A-Z0-9_]+: [^\r\n\0]*$/.test(message),
    'E_COMPILER_ABI: invalid diagnostic format');
  assert.ok(message.startsWith(expectedCode + ': '),
    'wasm32 compiler returned a different diagnostic code');
  assert.equal(stderr, sourceLabel + ': ' + wasmDiagnostic.line +
    ':' + wasmDiagnostic.col + ': ' + message + '\n',
    'native attoc and wasm32 compiler diagnostics are not byte-identical');
  return {code: expectedCode, line: wasmDiagnostic.line, col: wasmDiagnostic.col, message};
}

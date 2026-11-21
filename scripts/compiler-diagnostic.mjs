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

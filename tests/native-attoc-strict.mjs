// Fail-closed native CLI runner for browser/Wasm compiler parity tests.
// An infrastructure failure, signal, or malformed diagnostic is never a
// valid source rejection. Keep stderr byte parity on negative previews.
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { verifyCompilerDiagnosticParity } from '../scripts/compiler-diagnostic.mjs';

export function nativeAttocBytes(attoc, sourcePath, wasmFailure = null, { timeoutMs = 20_000 } = {}) {
  assert.ok(typeof attoc === 'string' && attoc.length > 0, 'native attoc path required');
  assert.ok(typeof sourcePath === 'string' && sourcePath.length > 0, 'native source path required');
  const result = spawnSync(attoc, [sourcePath], {
    shell: false, timeout: timeoutMs, maxBuffer: 1 << 22,
  });
  assert.ifError(result.error);
  assert.equal(result.signal, null, `${sourcePath}: native attoc terminated by signal`);
  assert.ok(result.status === 0 || result.status === 1,
    `${sourcePath}: unexpected native attoc exit ${String(result.status)} (expected 0 or 1)`);
  assert.ok(Buffer.isBuffer(result.stdout) && Buffer.isBuffer(result.stderr),
    `${sourcePath}: native attoc failed to produce captured buffers`);
  if (result.status === 0) {
    assert.equal(wasmFailure, null, `${sourcePath}: native attoc accepted rejected Wasm source`);
    assert.equal(result.stderr.length, 0, `${sourcePath}: successful native attoc wrote stderr`);
    assert.ok(WebAssembly.validate(result.stdout), `${sourcePath}: native attoc emitted invalid Wasm`);
    return new Uint8Array(result.stdout);
  }
  assert.equal(result.stdout.length, 0, `${sourcePath}: rejected native source emitted bytes`);
  const stderr = new TextDecoder('utf-8', {fatal: true}).decode(result.stderr);
  if (wasmFailure !== null) {
    verifyCompilerDiagnosticParity(stderr, sourcePath, {
      ok: false, message: wasmFailure.message,
      line: wasmFailure.line, col: wasmFailure.col,
    }, wasmFailure.code);
  } else {
    // Non-preview conformance checks may not supply a Wasm diagnostic.
    // They must still prove a real compiler diagnostic, not missing binary,
    // timeout, usage error (exit 2), empty stderr, or arbitrary nonzero exit.
    assert.ok(stderr.startsWith(sourcePath + ': '),
      `${sourcePath}: rejection must identify its input source`);
    assert.match(stderr.slice(sourcePath.length + 2),
      /^(?:(?:[1-9]\d*):(?:[1-9]\d*): )?E_[A-Z0-9_]+: [^\r\n\0]+\n$/,
      `${sourcePath}: malformed native diagnostic`);
  }
  return null;
}

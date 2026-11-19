// Canonical v3 signed-i64 guest comparison for native and wasm32 Rust compilers.
// A host-side BigInt oracle alone is never a passing Atto guest test.
import test from 'node:test';
import assert from 'node:assert/strict';
import {createHash} from 'node:crypto';
import {spawnSync} from 'node:child_process';
import {readFileSync, writeFileSync, mkdtempSync, existsSync, rmSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {join} from 'node:path';

const manifestBytes = readFileSync(new URL('../contracts/parity/portable-scalar-v3-20261101.json', import.meta.url));
const manifest = JSON.parse(manifestBytes.toString('utf8'));
const canonicalBlob = 'cd51c7320812d1ee032a211890cb427c984b2ea2';
const blobSha = bytes => createHash('sha1').update('blob ' + bytes.length + '\0').update(bytes).digest('hex');
const attoc = process.env.ATTOC;
assert.ok(attoc, 'ATTOC must name a built native Atto compiler; missing compiler is not a skip');
const compilerBytes = readFileSync(new URL('../target/wasm32-unknown-unknown/release/atto_compiler.wasm', import.meta.url));
assert.ok(WebAssembly.validate(compilerBytes), 'real wasm32 Atto compiler required');
const compiler = new WebAssembly.Instance(new WebAssembly.Module(compilerBytes), {}).exports;
assert.equal(compiler.abi_version(), 2, 'unreviewed browser compiler ABI');
const decoder = new TextDecoder('utf-8', {fatal:true});

function wasmCompile(source) {
  const input = Buffer.from(source, 'utf8');
  assert.ok(input.length && input.length <= compiler.input_capacity(), 'compiler source budget');
  const ptr = compiler.input_ptr();
  assert.ok(ptr >= 0 && ptr + input.length <= compiler.memory.buffer.byteLength, 'input pointer');
  new Uint8Array(compiler.memory.buffer, ptr, input.length).set(input);
  const status = compiler.compile(input.length);
  if (status !== 0) {
    assert.equal(compiler.result_length(), 0, 'rejection emitted Wasm');
    const memory = new Uint8Array(compiler.memory.buffer);
    const start = compiler.error_ptr();
    assert.ok(start >= 0 && start < memory.length, 'diagnostic pointer');
    let end = start;
    while (end < Math.min(memory.length, start + 512) && memory[end] !== 0) end++;
    return {ok:false, diagnostic:decoder.decode(memory.subarray(start, end))};
  }
  const start = compiler.result_ptr(), length = compiler.result_length();
  assert.ok(length >= 8 && length <= 1048576 && start >= 0 &&
    start + length <= compiler.memory.buffer.byteLength, 'output bounds');
  const wasm = new Uint8Array(compiler.memory.buffer, start, length).slice();
  assert.ok(WebAssembly.validate(wasm), 'browser compiler output is not Wasm');
  return {ok:true, wasm};
}
function executeGuest(bytes) {
  const mod = new WebAssembly.Module(bytes);
  assert.deepEqual(WebAssembly.Module.imports(mod), [], 'scalar guests must import no host capabilities');
  const instance = new WebAssembly.Instance(mod, {});
  assert.equal(typeof instance.exports.main, 'function');
  return instance.exports.main();
}

test('scalar-v3 contract is the unchanged Cranelift/Wasm Git blob', () => {
  assert.equal(blobSha(manifestBytes), canonicalBlob);
  assert.equal(manifest.schema_version, '2026-11-01.scalar-v3');
  assert.equal(manifest.scope, 'import-free-pure-i64');
  assert.deepEqual([manifest.positive.length,manifest.compile_reject.length,manifest.checked_overflow.length], [12,5,4]);
  const all = [...manifest.positive, ...manifest.compile_reject, ...manifest.checked_overflow];
  assert.equal(new Set(all.map(x => x.id)).size, 21);
  for (const row of all) {
    assert.match(row.id, /^[a-z0-9]+(?:-[a-z0-9]+)*$/);
    assert.match(row.expression, /^[0-9+\-*() \t]+$/);
    assert.ok(row.expression.length < 200);
  }
});

for (const [kind,rows] of Object.entries({
  positive:manifest.positive, compile_reject:manifest.compile_reject, checked_overflow:manifest.checked_overflow
})) {
  for (const row of rows) {
    test('real native/wasm32 compiler + Wasm guest scalar-v3 ' + kind + ' ' + row.id, () => {
      const directory = mkdtempSync(join(tmpdir(), 'atto-four-backend-v3-'));
      try {
        const source = 'pub fnc main(): int { return ' + row.expression + '; }\n';
        const sourcePath = join(directory, 'main.atto');
        const outputPath = join(directory, 'guest.wasm');
        writeFileSync(sourcePath, source, {flag:'wx'});
        const native = spawnSync(attoc, [sourcePath, '-o', outputPath], {
          timeout:10000, maxBuffer:2 << 20, encoding:'utf8'
        });
        assert.equal(native.error, undefined, 'native compiler failed to start or timed out');
        assert.equal(native.signal, null, 'native compiler terminated abnormally');
        const browser = wasmCompile(source);
        if (kind === 'compile_reject') {
          assert.equal(native.status, 1, 'native compiler must reject in source phase');
          assert.equal(native.stdout, '', 'negative compiled output on stdout');
          assert.equal(existsSync(outputPath), false, 'rejected source published guest artifact');
          assert.match(native.stderr, /E_[A-Z0-9_]+/);
          assert.equal(browser.ok, false, 'wasm32 compiler incorrectly admitted rejected source');
          assert.match(browser.diagnostic, /E_[A-Z0-9_]+/);
          return;
        }
        assert.equal(native.status, 0, 'native compiler rejected: ' + native.stderr);
        assert.equal(browser.ok, true, 'wasm32 compiler rejected: ' + browser.diagnostic);
        const nativeWasm = readFileSync(outputPath);
        assert.deepEqual(nativeWasm, Buffer.from(browser.wasm), 'native and browser compiler emitted different Wasm');
        if (kind === 'positive') {
          assert.equal(executeGuest(nativeWasm), BigInt(row.expected), 'native guest value mismatch');
          assert.equal(executeGuest(browser.wasm), BigInt(row.expected), 'browser compiler guest value mismatch');
        } else {
          assert.throws(() => executeGuest(nativeWasm), WebAssembly.RuntimeError,
            'checked overflow must trap in actual guest, not wrap or compile-reject');
          assert.throws(() => executeGuest(browser.wasm), WebAssembly.RuntimeError);
        }
      } finally {
        rmSync(directory, {recursive:true,force:true});
      }
    });
  }
}

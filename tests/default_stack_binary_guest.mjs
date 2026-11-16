// Execute actual Atto guest Wasm from both native and wasm32 compilers.
// This is a source-order and checked-operation regression, not a text check.
import assert from 'node:assert/strict';
import {readFileSync, writeFileSync, mkdtempSync, rmSync} from 'node:fs';
import {spawnSync} from 'node:child_process';
import {tmpdir} from 'node:os';
import {join} from 'node:path';

const compiler = process.env.ATTOC || 'target/release/attoc';
const compilerWasm = process.env.ATTO_COMPILER_WASM ||
  'target/wasm32-unknown-unknown/release/atto_compiler.wasm';
const folder = mkdtempSync(join(tmpdir(), 'atto-binary-guest-'));
const api = new WebAssembly.Instance(
  new WebAssembly.Module(readFileSync(compilerWasm)), {}
).exports;
assert.equal(api.abi_version(), 2);
const decoder = new TextDecoder();
const encoder = new TextEncoder();

function compileNative(source) {
  const path = join(folder, 'source.atto');
  writeFileSync(path, source, 'utf8');
  const child = spawnSync(compiler, [path], {maxBuffer: 4 << 20});
  assert.notEqual(child.status, null, child.stderr.toString());
  return {ok: child.status === 0, status: child.status, wasm: child.stdout,
    diagnostic: child.stderr.toString()};
}

function compileBrowser(source) {
  const data = encoder.encode(source);
  assert.ok(data.length <= api.input_capacity());
  new Uint8Array(api.memory.buffer, api.input_ptr(), data.length).set(data);
  const status = api.compile(data.length);
  if (status !== 0) {
    assert.equal(status, 1);
    assert.equal(api.result_length(), 0, 'rejection must emit no Wasm');
    const mem = new Uint8Array(api.memory.buffer);
    let end = api.error_ptr();
    while (mem[end] !== 0) end++;
    return {ok: false, diagnostic: decoder.decode(mem.subarray(api.error_ptr(), end))};
  }
  return {ok: true, wasm:
    new Uint8Array(api.memory.buffer, api.result_ptr(), api.result_length()).slice()};
}

function guestResult(wasm) {
  assert.ok(WebAssembly.validate(wasm), 'real valid guest Wasm');
  const mod = new WebAssembly.Module(wasm);
  assert.deepEqual(WebAssembly.Module.imports(mod), [], 'pure arithmetic has no imports');
  const instance = new WebAssembly.Instance(mod, {});
  return instance.exports.main();
}

function accepts(label, source, expected) {
  const native = compileNative(source);
  assert.equal(native.ok, true, label + ': ' + native.diagnostic);
  const browser = compileBrowser(source);
  assert.equal(browser.ok, true, label + ': ' + browser.diagnostic);
  assert.deepEqual(Buffer.from(browser.wasm), native.wasm, label + ': compiler bytes differ');
  assert.equal(guestResult(native.wasm), expected, label + ': wrong arithmetic order/result');
  console.log('PASS guest binary ' + label);
}

try {
  accepts('250 left-associated additions',
    'pub fnc main(): int { return ' + Array(250).fill('1').join(' + ') + '; }', 250n);
  accepts('subtraction preserves left association',
    'pub fnc main(): int { return 100 - 20 - 3; }', 77n);
  accepts('multiplication precedence',
    'pub fnc main(): int { return 20 - 3 * 4; }', 8n);
  accepts('mixed addition precedence',
    'pub fnc main(): int { return 1 + 2 * 3; }', 7n);

  for (const op of [' + ', ' * ', ' - ', ' == ', ' < ']) {
    const source = 'pub fnc main(): int { return ' + Array(3000).fill('1').join(op) + '; }';
    const native = compileNative(source);
    const browser = compileBrowser(source);
    assert.equal(native.ok, false, op + ' native unexpectedly compiled');
    assert.equal(native.status, 1);
    assert.equal(native.wasm.length, 0, op + ' native emitted an artifact');
    assert.match(native.diagnostic, /E_NESTING/, op);
    assert.equal(browser.ok, false, op + ' browser unexpectedly compiled');
    assert.match(browser.diagnostic, /E_NESTING/, op);
  }
  console.log('PASS binary 3000-term chains fail closed on both compilers');
} finally {
  rmSync(folder, {recursive: true, force: true});
}

// Focused executable borrow conformance for the stacked Borrow<T> compiler.
// Unlike the full suite, this does not depend on unrelated long-expression
// stress limits. Never use this result to claim the full suite passed.
//
// Requires:
//   cargo build --locked --release --bin attoc
//   cargo rustc --locked --release --lib --target wasm32-unknown-unknown \
//     --crate-type cdylib -- -C link-arg=--max-memory=67108864
//   ATTOC=target/release/attoc node tests/borrow_clarity_guest.mjs
import assert from 'node:assert/strict';
import { readFileSync, readdirSync } from 'node:fs';
import { resolve, dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { spawnSync } from 'node:child_process';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const nativePath = resolve(root, process.env.ATTOC || 'target/release/attoc');
const browserCompilerPath = resolve(root,
  'target/wasm32-unknown-unknown/release/atto_compiler.wasm');
const encoder = new TextEncoder();
const decoder = new TextDecoder('utf-8', { fatal: true });

const compilerBytes = readFileSync(browserCompilerPath);
assert.equal(WebAssembly.validate(compilerBytes), true, 'compiler is real Wasm');
const compilerModule = new WebAssembly.Module(compilerBytes);
assert.deepEqual(WebAssembly.Module.imports(compilerModule), [], 'compiler must have no imports');
const { exports: api } = new WebAssembly.Instance(compilerModule, {});
assert.equal(api.abi_version(), 2, 'expected compiler ABI v2');

function compile(source) {
  const bytes = encoder.encode(source);
  assert.ok(bytes.length <= api.input_capacity(), 'fixture fits compiler input capacity');
  new Uint8Array(api.memory.buffer, api.input_ptr(), bytes.length).set(bytes);
  const status = api.compile(bytes.length);
  if (status !== 0) {
    assert.equal(api.result_length(), 0, 'failed compile emitted zero guest bytes');
    const mem = new Uint8Array(api.memory.buffer);
    let end = api.error_ptr();
    while (mem[end]) end++;
    const msg = decoder.decode(mem.subarray(api.error_ptr(), end));
    return { ok: false, code: msg.split(':')[0], message: msg,
      line: api.error_line_number(), col: api.error_column_number() };
  }
  const wasm = new Uint8Array(api.memory.buffer, api.result_ptr(), api.result_length()).slice();
  assert.equal(WebAssembly.validate(wasm), true, 'compiled guest is valid Wasm');
  return { ok: true, wasm };
}

function runGuest(wasm) {
  const module = new WebAssembly.Module(wasm);
  const allow = new Set(['print_i64', 'print_bool', 'print_str', 'print_newline', 'print_f64']);
  for (const item of WebAssembly.Module.imports(module)) {
    assert.equal(item.module, 'atto', 'only approved guest import namespace');
    assert.ok(allow.has(item.name), 'no actor, FFI, or host capability in borrow fixture');
  }
  let instance;
  let stdout = '';
  const atto = {
    print_i64: v => { stdout += String(v); },
    print_bool: v => { stdout += v ? 'true' : 'false'; },
    print_newline: () => { stdout += '\n'; },
    print_f64: v => { stdout += String(v); },
    print_str: (offset, len) => {
      stdout += decoder.decode(new Uint8Array(instance.exports.memory.buffer, offset, len));
    },
  };
  instance = new WebAssembly.Instance(module, { atto });
  return { value: instance.exports.main(), stdout };
}

function header(source, key) {
  const prefix = '// ' + key + ': ';
  return source.split('\n').filter(line => line.startsWith(prefix))
    .map(line => line.slice(prefix.length));
}

const results = { pass: 0, fail: 0, nativeMatched: 0 };
for (const kind of ['pass', 'fail']) {
  const dir = join(root, 'tests', 'fixtures', kind);
  const names = readdirSync(dir).filter(n => n.startsWith('borrow-clarity-') && n.endsWith('.atto')).sort();
  assert.equal(names.length, kind === 'pass' ? 7 : 10, 'test registry cardinality');
  for (const name of names) {
    const file = join(dir, name);
    const source = readFileSync(file, 'utf8');
    const wasmCompiler = compile(source);
    const native = spawnSync(nativePath, [file], { maxBuffer: 1 << 22 });
    if (native.error) throw native.error;
    const label = kind + '/' + name;
    if (kind === 'pass') {
      assert.equal(wasmCompiler.ok, true, label + ': ' + (wasmCompiler.message || 'compile failed'));
      assert.equal(native.status, 0, label + ' native compiler failed: ' + native.stderr.toString());
      assert.deepEqual(Buffer.from(wasmCompiler.wasm), native.stdout,
        label + ' native/browser compiler guest-byte parity');
      const another = compile(source);
      assert.equal(another.ok, true, label + ' deterministic recompile');
      assert.deepEqual(another.wasm, wasmCompiler.wasm, label + ' deterministic wasm bytes');
      const guest = runGuest(wasmCompiler.wasm);
      assert.equal(guest.value, BigInt(header(source, 'expect-exit')[0]), label + ' guest result');
      const wantStdout = header(source, 'expect-stdout');
      assert.equal(guest.stdout, wantStdout.length ? wantStdout.join('\n') + '\n' : '',
        label + ' guest stdout');
      results.pass++;
    } else {
      const want = header(source, 'expect-error')[0];
      assert.equal(wasmCompiler.ok, false, label + ' unexpectedly compiled');
      assert.equal(wasmCompiler.code, want, label + ' diagnostic: ' + wasmCompiler.message);
      assert.ok(wasmCompiler.line >= 1 && wasmCompiler.col >= 1, label + ' diagnostic position');
      assert.notEqual(native.status, 0, label + ' native compiler accepted rejected source');
      assert.equal(native.stdout.length, 0, label + ' native compiler emitted forbidden guest bytes');
      assert.ok(native.stderr.toString().includes(want), label + ' native diagnostic missing ' + want);
      results.fail++;
    }
    results.nativeMatched++;
    console.log('BORROW_CLARITY_CASE_PASS ' + label);
  }
}
assert.deepEqual(results, { pass: 7, fail: 10, nativeMatched: 17 });
console.log('BORROW_CLARITY_GUEST_PASS ' + JSON.stringify(results)
  + ' verified native and wasm32 compilers; guest execution for positives; no guest bytes on negatives');

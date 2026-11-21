// A real double-Wasm test: Rust compiler compiled to wasm32, then it
// emits genuine guest Wasm which Node validates and executes. NO JS parser,
// evaluator, transpiler, substitute compiler, or synthetic guest.
import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile, writeFile, mkdtemp, rm } from 'node:fs/promises';
import { execFileSync } from 'node:child_process';
import { spawnSync } from 'node:child_process';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { createHash } from 'node:crypto';
import { fileURLToPath } from 'node:url';

const root = fileURLToPath(new URL('../', import.meta.url));
const compilerPath = process.env.ATTO_RUST_WASM_COMPILER ??
  fileURLToPath(new URL('../target/wasm32-unknown-unknown/release/atto_compiler.wasm', import.meta.url));
const nativeCli = process.env.ATTO_RUST_NATIVE_CLI ??
  fileURLToPath(new URL('../target/release/attoc', import.meta.url));
const sourceHead = execFileSync('git', ['rev-parse', 'HEAD'], {cwd: root, encoding: 'utf8'}).trim();
const expected = process.env.EXPECTED_SOURCE_SHA;
if (expected) {
  assert.match(expected, /^[0-9a-f]{40}$/);
  assert.equal(sourceHead, expected, 'exact head must match the tested source');
}
const compilerBytes = await readFile(compilerPath);
assert.ok(compilerBytes.length > 1000, 'genuine compiled Rust-to-Wasm tool is required');
assert.equal(WebAssembly.validate(compilerBytes), true, 'Rust compiler itself must be valid Wasm');
const compilerModule = new WebAssembly.Module(compilerBytes);
assert.deepEqual(WebAssembly.Module.imports(compilerModule), [],
  'browser compiler may not require ambient host imports');
const compiler = new WebAssembly.Instance(compilerModule, {});
const api = compiler.exports;
assert.ok(api.memory instanceof WebAssembly.Memory);
for (const fn of ['abi_version','input_capacity','input_ptr','compile','result_ptr',
 'result_length','error_ptr','error_line_number','error_column_number']) {
  assert.equal(typeof api[fn], 'function', 'missing browser compiler ABI: ' + fn);
}
assert.equal(api.abi_version(), 2);
const capacity = api.input_capacity();
assert.ok(Number.isInteger(capacity) && capacity > 0 && capacity <= 65536);
const decoder = new TextDecoder('utf-8', {fatal: true});
const encoder = new TextEncoder();
const hash = bytes => createHash('sha256').update(bytes).digest('hex');

function checkedSlice(ptr, length, limit) {
  const mem = api.memory.buffer;
  if (!Number.isSafeInteger(ptr) || !Number.isSafeInteger(length) ||
      ptr < 0 || length < 0 || length > limit ||
      ptr > mem.byteLength || length > mem.byteLength - ptr) {
    throw new Error('Rust compiler ABI memory bounds violation');
  }
  return new Uint8Array(mem, ptr, length);
}

function compileViaBrowserAbi(source) {
  const encoded = encoder.encode(source);
  assert.ok(encoded.length <= capacity);
  checkedSlice(api.input_ptr(), encoded.length, capacity).set(encoded);
  const status = api.compile(encoded.length);
  assert.ok(status === 0 || status === 1, 'unknown Rust compiler status ' + status);
  if (status !== 0) {
    assert.equal(api.result_length(), 0, 'failed compilation published stale Wasm');
    const ptr = api.error_ptr();
    const messageBytes = checkedSlice(ptr, 512, 512);
    const terminator = messageBytes.indexOf(0);
    assert.ok(terminator > 0, 'missing NUL-terminated compiler diagnostic');
    const message = decoder.decode(messageBytes.subarray(0, terminator));
    assert.match(message, /E_[A-Z0-9_]+/);
    assert.ok(api.error_line_number() > 0);
    assert.ok(api.error_column_number() > 0);
    return {ok:false,message};
  }
  const length = api.result_length();
  assert.ok(length >= 8 && length <= 1024 * 1024, 'guest Wasm output budget');
  const bytes = checkedSlice(api.result_ptr(), length, 1024 * 1024).slice();
  assert.equal(WebAssembly.validate(bytes), true, 'compiler emitted malformed Wasm');
  return {ok:true,bytes};
}

function realGuestExit42(bytes, label) {
  const mod = new WebAssembly.Module(bytes);
  assert.deepEqual(WebAssembly.Module.imports(mod), [], label + ': undeclared guest imports');
  const guest = new WebAssembly.Instance(mod, {});
  assert.equal(typeof guest.exports.main, 'function', label + ': missing main()');
  assert.equal(guest.exports.main(), 42n, label + ': real i64 main() return');
}
const positives = [
 'lambda-direct-int.atto',
 'lambda-direct-zero.atto',
 'lambda-direct-two-args.atto',
 '165-callable-five-function-forms.atto',
];
const negatives = [
 ['lambda-direct-arity.atto', 'E_ARITY'],
 ['lambda-direct-capture.atto', 'E_CAPTURE'],
 ['lambda-direct-escape.atto', 'E_UNSUPPORTED']
];

test('current exact head: compiled Rust compiler Wasm executes four independent Atto guests', async () => {
  console.log('source_head=' + sourceHead + ' compiler_wasm_sha256=' + hash(compilerBytes));
  for (const name of positives) {
    const source = await readFile(new URL('./fixtures/pass/' + name, import.meta.url), 'utf8');
    const compiled = compileViaBrowserAbi(source);
    assert.equal(compiled.ok, true, name + ': ' + (compiled.message ?? 'failed'));
    realGuestExit42(compiled.bytes, name);
    const repeat = compileViaBrowserAbi(source);
    assert.equal(repeat.ok, true);
    assert.equal(hash(compiled.bytes), hash(repeat.bytes), name + ': non-deterministic guest output');
    console.log(name + ' guest_sha256=' + hash(compiled.bytes) + ' exit=42');
  }
});

test('actual Rust compiler rejects unsafe lambda cases with zero guest bytes and recovers', async () => {
  const positiveSource = await readFile(new URL('./fixtures/pass/lambda-direct-int.atto', import.meta.url), 'utf8');
  for (const [name, code] of negatives) {
    const source = await readFile(new URL('./fixtures/fail/' + name, import.meta.url), 'utf8');
    const result = compileViaBrowserAbi(source);
    assert.equal(result.ok, false, name);
    assert.ok(result.message.includes(code), name + ': ' + result.message);
    const recovered = compileViaBrowserAbi(positiveSource);
    assert.equal(recovered.ok, true, 'compiler not reusable after diagnostic ' + name);
    realGuestExit42(recovered.bytes, 'recovered ' + name);
  }
});

test('four typed void lambda spellings execute in BOTH Rust-native and Rust-wasm32 compiler', async () => {
  // Contract: type-first, name-first, explicit arrow void, explicit colon void.
  // This PR already compiles these via Rust tests; this asserts ACTUAL guests.
  const headers = [
    ['type-first', '|int foo, string x| -> { return; };'],
    ['name-first', '|foo: int, x: string| -> { return; }'],
    ['arrow-void', '|foo: int, x: string| -> void { return; }'],
    ['colon-void', '|foo: int, x: string| : void -> { return; }'],
  ];
  const scratch = await mkdtemp(join(tmpdir(), 'atto-typed-lambda-real-'));
  try {
    for (const [label, header] of headers) {
      const source = [
        'pub fnc main(): int {',
        '  const v = ' + header,
        '  v(5, "ok");',
        '  return 42;',
        '}',
      ].join('\\n').replaceAll('\\n','\n');
      const compiled = compileViaBrowserAbi(source);
      assert.equal(compiled.ok, true, label + ': ' + (compiled.message ?? 'failed'));
      realGuestExit42(compiled.bytes, 'browser typed ' + label);
      const input = join(scratch, label + '.atto');
      const output = join(scratch, label + '.wasm');
      await writeFile(input, source);
      const result = spawnSync(nativeCli, [input, '-o', output], {encoding: 'utf8', timeout: 30000});
      assert.equal(result.status, 0, label + ': native compiler: ' + (result.stderr || result.error));
      realGuestExit42(await readFile(output), 'native typed ' + label);
      console.log('typed-lambda-' + label + ' browser+native exit=42');
    }
    for (const [label, source, code] of [
      ['void-return-value',
       'pub fnc main(): int { const v = |foo: int| : void -> { return 7; }; v(5); return 42; }',
       'E_TYPE'],
      ['void-parameter',
       'pub fnc main(): int { const v = |void foo| : void -> { return; }; v(5); return 42; }',
       'E_TYPE'],
    ]) {
      const failed = compileViaBrowserAbi(source);
      assert.equal(failed.ok, false, label);
      assert.ok(failed.message.includes(code), label + ': ' + failed.message);
    }
  } finally {
    await rm(scratch, {recursive: true, force: true});
  }
});

test('native Rust attoc and the wasm32 Rust compiler agree on executed guest results', async () => {
  const scratch = await mkdtemp(join(tmpdir(), 'atto-real-double-wasm-'));
  try {
    for (const name of positives) {
      const input = fileURLToPath(new URL('./fixtures/pass/' + name, import.meta.url));
      const output = join(scratch, name + '.wasm');
      const result = spawnSync(nativeCli, [input, '-o', output], {encoding:'utf8',timeout:30000});
      assert.equal(result.status, 0, name + ': native CLI failed: ' + (result.stderr || result.error));
      const binary = await readFile(output);
      assert.equal(WebAssembly.validate(binary), true);
      realGuestExit42(binary, 'native attoc ' + name);
      console.log(name + ' native_sha256=' + hash(binary) + ' exit=42');
    }
    for (const [name, code] of negatives) {
      const input = fileURLToPath(new URL('./fixtures/fail/' + name, import.meta.url));
      const output = join(scratch, name + '.wasm');
      const result = spawnSync(nativeCli, [input, '-o', output], {encoding:'utf8',timeout:30000});
      assert.notEqual(result.status, 0, 'negative compiled in native CLI: ' + name);
      assert.ok(result.stderr?.includes(code), name + ': missing native diagnostic ' + code);
    }
  } finally {
    await rm(scratch, {recursive:true,force:true});
  }
});

test('pipe-delimited const fnc compiles through real Rust/Wasm and native Rust, executes and recovers', async () => {
  const valid = [
    ['ignored-int', 'pub fnc main(): int { const fnc v = |_| : int -> 5; return v(41); }', 5n],
    ['ignored-bool', 'pub fnc main(): int { const fnc v = |_| : int -> 5; return v(true); }', 5n],
    ['zero-arg', 'pub fnc main(): int { const fnc v = || : int -> 5; return v(); }', 5n],
    ['typed-module', 'const fnc v = |int _| : int -> 5; pub fnc main(): int { return v(41); }', 5n],
    ['local-named', 'pub fnc main(): int { const fnc v = |int x| : int -> x + 1; return v(x: 41); }', 42n],
  ];
  const invalid = [
    'pub fnc main(): int { const fnc v = _ : int -> 5; return 0; }',
    'pub fnc main(): int { const fnc v = _ -> 5; return 0; }',
    'pub fnc main(): int { const fnc v = x : int -> x + 5; return 0; }',
    'pub fnc main(): int { const fnc v = |_| : int -> 5; return v(); }',
  ];
  const scratch = await mkdtemp(join(tmpdir(), 'atto-pipe-const-real-'));
  try {
    for (const [label, source, expectedExit] of valid) {
      const result = compileViaBrowserAbi(source);
      assert.equal(result.ok, true, label + ': ' + (result.message ?? 'compiler failed'));
      const browserModule = new WebAssembly.Module(result.bytes);
      assert.deepEqual(WebAssembly.Module.imports(browserModule), [], label);
      assert.equal(new WebAssembly.Instance(browserModule, {}).exports.main(), expectedExit, label);
      const repeat = compileViaBrowserAbi(source);
      assert.equal(repeat.ok, true, label + ' nonrepeatable');
      assert.equal(hash(result.bytes), hash(repeat.bytes), label + ' nondeterministic output');
      const input = join(scratch, label + '.atto');
      const output = join(scratch, label + '.wasm');
      await writeFile(input, source);
      const native = spawnSync(nativeCli, [input, '-o', output], { encoding: 'utf8', timeout: 30000 });
      assert.equal(native.status, 0, label + ': native compiler: ' + (native.stderr || native.error));
      const nativeModule = new WebAssembly.Module(await readFile(output));
      assert.deepEqual(WebAssembly.Module.imports(nativeModule), [], label + ' native');
      assert.equal(new WebAssembly.Instance(nativeModule, {}).exports.main(), expectedExit, label + ' native');
    }

    for (const [i, source] of invalid.entries()) {
      const rejected = compileViaBrowserAbi(source);
      assert.equal(rejected.ok, false, 'rejected bare/invalid head ' + i);
      assert.match(rejected.message, /E_[A-Z0-9_]+/);
      // compileViaBrowserAbi asserts result_length() is zero on all failures.
      const input = join(scratch, 'invalid-' + i + '.atto');
      const output = join(scratch, 'invalid-' + i + '.wasm');
      await writeFile(input, source);
      const native = spawnSync(nativeCli, [input, '-o', output], { encoding: 'utf8', timeout: 30000 });
      assert.notEqual(native.status, 0, 'native accepted invalid const lambda ' + i);
      const recovered = compileViaBrowserAbi(valid[0][1]);
      assert.equal(recovered.ok, true, 'Rust/Wasm compiler did not recover after rejection ' + i);
    }
  } finally {
    await rm(scratch, { recursive: true, force: true });
  }
});

// Exact compiler-to-guest parity for readonly inline self receivers.
// Must execute native attoc and the real wasm32-hosted Rust compiler;
// no JavaScript source-language simulation is accepted as proof.
import assert from 'node:assert/strict';
import {readFile, writeFile, mkdtemp, rm, stat} from 'node:fs/promises';
import {spawnSync} from 'node:child_process';
import {fileURLToPath} from 'node:url';
import {join} from 'node:path';
import {tmpdir} from 'node:os';

const here = new URL('.', import.meta.url);
const attoc = process.env.ATTOC || fileURLToPath(new URL('../target/debug/attoc', here));
const compilerPath = fileURLToPath(new URL('../target/wasm32-unknown-unknown/release/atto_compiler.wasm', here));
const compilerBytes = await readFile(compilerPath);
assert.ok(WebAssembly.validate(compilerBytes), 'real wasm32 compiler artifact required');
const compilerModule = new WebAssembly.Module(compilerBytes);
assert.deepEqual(WebAssembly.Module.imports(compilerModule), [], 'browser compiler cannot import ambient host capabilities');
const {exports: api} = new WebAssembly.Instance(compilerModule, {});
assert.equal(api.abi_version(), 2, 'expected public compiler ABI v2');
for (const key of ['memory','input_ptr','input_capacity','compile','result_ptr',
                   'result_length','error_ptr','error_line_number','error_column_number']) {
  assert.ok(key in api, 'missing real compiler export: '+key);
}
const encoder = new TextEncoder();
const decoder = new TextDecoder('utf8', {fatal: true});
function browserCompile(source) {
  const input = encoder.encode(source);
  const ptr = api.input_ptr(), capacity = api.input_capacity();
  assert.ok(Number.isSafeInteger(ptr) && Number.isSafeInteger(capacity) && ptr >= 0 && capacity > 0);
  assert.ok(input.length <= capacity && ptr + input.length <= api.memory.buffer.byteLength,
    'source too large for browser compiler input memory');
  new Uint8Array(api.memory.buffer, ptr, input.length).set(input);
  const rc = api.compile(input.length);
  if (rc !== 0) {
    assert.equal(api.result_length(), 0, 'failed compile returned stale guest bytes');
    const bytes = new Uint8Array(api.memory.buffer);
    const start = api.error_ptr();
    assert.ok(Number.isSafeInteger(start) && start >= 0 && start < bytes.length,
      'invalid compiler diagnostic pointer');
    let end = start;
    while (end < bytes.length && end - start < 512 && bytes[end] !== 0) end++;
    assert.ok(end < bytes.length && end - start < 512 && bytes[end] === 0,
      'unterminated compiler diagnostic');
    const message = decoder.decode(bytes.subarray(start, end));
    assert.match(message, /^E_[A-Z_]+:/, 'structured compiler error required');
    assert.ok(api.error_line_number() >= 1 && api.error_column_number() >= 1,
      'compiler error requires source location');
    return {ok:false, code: message.split(':')[0], message};
  }
  const resultPtr = api.result_ptr(), resultLength = api.result_length();
  assert.ok(Number.isSafeInteger(resultPtr) && resultPtr >= 0 &&
    Number.isSafeInteger(resultLength) && resultLength > 8 && resultLength <= 1048576 &&
    resultPtr + resultLength <= api.memory.buffer.byteLength, 'invalid or oversized guest range');
  const guest = new Uint8Array(api.memory.buffer, resultPtr, resultLength).slice();
  assert.ok(WebAssembly.validate(guest), 'compiler emitted invalid guest Wasm');
  return {ok:true, guest};
}
const source = (header, call) =>
  'define class Calculator as\n    '+header+'\nend\n'+
  'pub fnc main(): int {\n  val c = new Calculator();\n  return '+call+';\n}\n';
const cases = [
  ['value-one',
    source('pub fnc value(int n): int { return n; }', 'c.value(42)'),
    source('pub fnc value(self, int n): int { return n; }', 'c.value(42)'), 42n],
  ['value-zero',
    source('pub fnc value(): int { return 7; }', 'c.value()'),
    source('pub fnc value(self): int { return 7; }', 'c.value()'), 7n],
];
const rejects = [
  ['static-self', source('static fnc value(self, int n): int { return n; }', 'c.value(42)'), 'E_UNSUPPORTED'],
  ['mutable-self', source('pub fnc value(mut self, int n): int { return n; }', 'c.value(42)'), 'E_UNSUPPORTED'],
  ['dangling-self', source('pub fnc value(self,): int { return 7; }', 'c.value()'), 'E_SYNTAX'],
  ['extra-call-argument', source('pub fnc value(self, int n): int { return n; }', 'c.value(c, 42)'), null],
];
const temp = await mkdtemp(join(tmpdir(), 'atto-inline-self-parity-'));
try {
  async function nativeCompile(src, label) {
    const input = join(temp, label+'.atto');
    const output = join(temp, label+'.wasm');
    await writeFile(input, src, 'utf8');
    const result = spawnSync(attoc, [input, '-o', output], {encoding:'utf8', timeout:30000, maxBuffer:1<<20});
    assert.equal(result.error, undefined, label+': native compiler process did not start');
    return {result, output, bytes:result.status===0 ? await readFile(output) : null};
  }
  function runGuest(bytes, want, label) {
    const module = new WebAssembly.Module(bytes);
    assert.deepEqual(WebAssembly.Module.imports(module), [], label+': unexpected guest host import');
    const instance = new WebAssembly.Instance(module, {});
    assert.equal(instance.exports.main(), want, label+': incorrect compiled guest result');
    assert.equal(instance.exports.panic_code.value, 0, label+': unexpected guest panic');
  }
  for (const [label, implicit, inline, want] of cases) {
    const browserImplicit = browserCompile(implicit);
    const browserInline = browserCompile(inline);
    assert.equal(browserImplicit.ok, true, label+': implicit form rejected');
    assert.equal(browserInline.ok, true, label+': inline form rejected');
    assert.deepEqual(browserInline.guest, browserImplicit.guest,
      label+': declaration-only self must not change Wasm ABI or value arity');
    for (const [kind, src, browser] of [['implicit', implicit, browserImplicit], ['inline', inline, browserInline]]) {
      const built = await nativeCompile(src, label+'-'+kind);
      assert.equal(built.result.status, 0, label+': native '+kind+' compile failed: '+built.result.stderr);
      assert.deepEqual(Buffer.from(browser.guest), built.bytes,
        label+': native and wasm32 compiler guest bytes differ for '+kind);
      runGuest(browser.guest, want, label+'-'+kind);
    }
  }
  for (const [label, src, expected] of rejects) {
    const browser = browserCompile(src);
    assert.equal(browser.ok, false, label+': malformed receiver produced executable guest');
    if (expected) assert.equal(browser.code, expected, label+': wrong compiler diagnostic');
    const built = await nativeCompile(src, 'negative-'+label);
    assert.equal(built.result.status, 1, label+': native compiler accepted invalid receiver');
    assert.match(built.result.stderr, new RegExp('\\b'+browser.code+'\\b'),
      label+': browser/native diagnostic code mismatch');
    await assert.rejects(stat(built.output), {code:'ENOENT'}, label+': rejected source emitted Wasm');
  }
  const recovery = source('pub fnc value(self, int n): int { return n; }', 'c.value(42)');
  const browser = browserCompile(recovery);
  assert.equal(browser.ok, true, 'compiler failed to recover after rejected receiver');
  const built = await nativeCompile(recovery, 'recovery');
  assert.equal(built.result.status, 0, 'native recovery compilation failed: '+built.result.stderr);
  assert.deepEqual(Buffer.from(browser.guest), built.bytes, 'post-failure compiler bytes drifted');
  runGuest(browser.guest, 42n, 'recovery');
  console.log('PASS: readonly inline self 2 receiver pairs, 4 executed native/wasm-identical guests, 4 negatives, post-failure recovery');
} finally {
  await rm(temp, {recursive:true, force:true});
}

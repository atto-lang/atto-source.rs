// Real source compiler -> executable guest Wasm, with native/wasm32 parity.
// NOT a parser-only acceptance test. Never mark PASS unless this script runs.
import assert from 'node:assert/strict';
import { mkdtemp, readFile, writeFile, rm } from 'node:fs/promises';
import { spawnSync } from 'node:child_process';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';

const attoc = process.env.ATTOC || fileURLToPath(new URL('../target/debug/attoc', import.meta.url));
const browserPath = fileURLToPath(new URL('../target/wasm32-unknown-unknown/release/atto_compiler.wasm', import.meta.url));
const compilerBytes = await readFile(browserPath);
assert.ok(WebAssembly.validate(compilerBytes), 'compiled wasm32 compiler binary is required');
const module = new WebAssembly.Module(compilerBytes);
assert.deepEqual(WebAssembly.Module.imports(module), [], 'compiler must be host-independent');
const api = new WebAssembly.Instance(module, {}).exports;
assert.equal(api.abi_version(), 2, 'browser compiler ABI');
const encoder = new TextEncoder();
const decoder = new TextDecoder('utf8', { fatal: true });

function browserCompile(source) {
  const bytes = encoder.encode(source);
  assert.ok(bytes.length < api.input_capacity(), 'source within compiler capacity');
  new Uint8Array(api.memory.buffer, api.input_ptr(), bytes.length).set(bytes);
  const result = api.compile(bytes.length);
  if (result !== 0) {
    assert.equal(api.result_length(), 0, 'negative compilation must emit no artifact');
    const memory = new Uint8Array(api.memory.buffer);
    const p = api.error_ptr();
    let end = p;
    while (end < p + 512 && memory[end] !== 0) end++;
    return { ok: false, error: decoder.decode(memory.subarray(p, end)) };
  }
  const guest = new Uint8Array(api.memory.buffer, api.result_ptr(), api.result_length()).slice();
  assert.ok(WebAssembly.validate(guest), 'browser compiler emitted invalid guest');
  return { ok: true, guest };
}

// Current-main integration proves the guard and caret forms, independently of unrelated operator changes.
const goodBodies = [
  '^;',
  '^ void;',
  'return void;',
  'if true { ^ }',
  'if true { ^ void; }',
  'if true { ^ void; } fi', // legacy explicit fi
  'do if true then ^ fi',
  'do if true then ^ void fi',
  'if true then if true { ^ void; } fi',
  'if true then if true { ^ void; } fi fi',
  'if true { if true then ^ void fi }',
  'if false { ^ void; }',
];
const endStyleFunctions = [
  'fnc process_explicit() -> void\n    guard false else ^ void end\nend',
  'fnc process_explicit() -> void\n    guard false { ^ void }\nend',
  'fnc process_explicit() -> void\n    guard true else ^ void end\nend',
  'fnc process_explicit() -> void\n    guard true { ^ void }\nend',
  'fnc overflow_if_called(int v): bool { return v + 1 > 0; }\nfnc process_explicit() -> void\n    guard false, overflow_if_called(9223372036854775807) else ^ void end\nend',
  'fnc overflow_if_called(int v): bool { return v + 1 > 0; }\nfnc process_explicit() -> void\n    guard false, overflow_if_called(9223372036854775807) { ^ void }\nend',
  'fnc overflow_if_called(int v): bool { return v + 1 > 0; }\nfnc process_explicit() -> void\n    guard true | overflow_if_called(9223372036854775807) else ^ void end\nend',
  'fnc overflow_if_called(int v): bool { return v + 1 > 0; }\nfnc process_explicit() -> void\n    guard true | overflow_if_called(9223372036854775807) { ^ void }\nend',
];
const goodSources = [
  ...goodBodies.map(body =>
    'fnc early_exit() -> void { ' + body +
    ' } pub fnc main(): int { early_exit(); ^ 42; }\n'),
  ...endStyleFunctions.map(fn =>
    fn + '\npub fnc main(): int { process_explicit(); ^ 42; }\n'),
];
const badSources = [
  'fnc process_explicit() -> int\n    guard false else ^ void end\nend\npub fnc main(): int { ^ process_explicit(); }',
  'fnc process_explicit() -> int\n    guard false { ^ void }\nend\npub fnc main(): int { ^ process_explicit(); }',
  'fnc process_explicit() -> void\n    guard false else val x = 1; end\nend\npub fnc main(): int { process_explicit(); ^ 42; }',
  'fnc process_explicit() -> void\n    guard false { val x = 1; }\nend\npub fnc main(): int { process_explicit(); ^ 42; }',
  'pub fnc main(): int { ^ void; }',
  'pub fnc main(): int { return void; }',
  'pub fnc main(): int { do if true then ^ void fi ^ 42; }',
  'pub fnc main(): int { if true { ^ void; } ^ 42; }',
  'pub fnc main(): int { val x = void; ^ 42; }',
  'fnc early_exit() -> void { ^ void + 1; } pub fnc main(): int { early_exit(); ^ 42; }',
];
const dir = await mkdtemp(join(tmpdir(), 'atto-void-caret-'));
try {
  for (const [i, source] of goodSources.entries()) {
    const fromBrowser = browserCompile(source);
    assert.equal(fromBrowser.ok, true, fromBrowser.error || source);
    const src = join(dir, 'pass-' + i + '.atto');
    const out = join(dir, 'pass-' + i + '.wasm');
    await writeFile(src, source);
    const native = spawnSync(attoc, [src, '-o', out], { encoding: 'utf8', timeout: 30000 });
    assert.equal(native.status, 0, source + ': ' + native.stderr);
    const nativeBytes = await readFile(out);
    assert.deepEqual(Buffer.from(fromBrowser.guest), nativeBytes,
      source + ': native/wasm32 compiler output divergence');
    const wasm = new WebAssembly.Module(nativeBytes);
    assert.deepEqual(WebAssembly.Module.imports(wasm), [], source + ': unexpected guest import');
    const guest = new WebAssembly.Instance(wasm, { atto: {} });
    assert.equal(guest.exports.main(), 42n, source + ': wrong guest return');
    assert.equal(guest.exports.panic_code.value, 0, source + ': guest panic');
  }
  for (const [i, source] of badSources.entries()) {
    const fromBrowser = browserCompile(source + '\n');
    assert.equal(fromBrowser.ok, false, source + ': browser compiler accepted invalid return');
    const src = join(dir, 'fail-' + i + '.atto');
    const out = join(dir, 'fail-' + i + '.wasm');
    await writeFile(src, source + '\n');
    const native = spawnSync(attoc, [src, '-o', out], { encoding: 'utf8', timeout: 30000 });
    assert.notEqual(native.status, 0, source + ': native compiler accepted invalid return');
    await assert.rejects(readFile(out), { code: 'ENOENT' },
      source + ': rejected compiler input left an executable artifact');
  }
  console.log('PASS void returns:', goodSources.length, 'native/browser Wasm guest byte parity + execution,',
    badSources.length, 'double-backend no-artifact rejections');
} finally {
  await rm(dir, { recursive: true, force: true });
}

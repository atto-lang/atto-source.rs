// Exact-head Rust -> Wasm guest gate. Does not certify trusted-native unsafe.
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { mkdtempSync, readFileSync, writeFileSync, existsSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { resolve, join } from 'node:path';

const attoc = resolve(process.env.ATTOC || 'target/release/attoc');
const cwd = mkdtempSync(join(tmpdir(), 'atto-unsafe-guest-'));
const write = (file, source) => writeFileSync(join(cwd, file), source + '\n');
const invoke = args => {
  const r = spawnSync(attoc, args, { cwd, encoding: 'utf8', timeout: 60_000, maxBuffer: 1 << 20 });
  assert.equal(r.error, undefined);
  assert.notEqual(r.status, null, 'compiler killed or timed out');
  return r;
};
const positive = [
  ['safe', 'pub fnc main(): int { return 42; }'],
  ['identifier', 'pub fnc main(): int { val int unsafe_tools = 42; return unsafe_tools; }'],
  ['comment', 'pub fnc main(): int { /* unsafe { } */ return 42; }'],
  ['string', 'pub fnc main(): int { val string s = "std/unsafe/ffi"; return 42; }'],
];
const negative = [
  ['empty', 'pub fnc main(): int { unsafe { } return 42; }'],
  ['nested', 'pub fnc main(): int { unsafe { unsafe { } } return 42; }'],
  ['lambda', 'pub fnc main(): int { val f = || -> { unsafe { } return 42; }; return 42; }'],
  ['defer', 'pub fnc main(): int { defer { unsafe { } }; return 42; }'],
  ['callable', 'pub unsafe fnc host_only(): int { return 42; } pub fnc main(): int { unsafe { return host_only(); } }'],
  ['native-package', 'import module innocent from "std/unsafe/ffi"; pub fnc main(): int { return 42; }'],
  ['native-package-alias', 'import module innocent from "ores.std.unsafe.ffi"; pub fnc main(): int { return 42; }'],
  ['imported-unsafe', 'import value from "./dep.atto"; pub fnc main(): int { return value(); }'],
];
let passed = 0, denied = 0;
try {
  write('dep.atto', 'pub fnc value(): int { unsafe { } return 42; }');
  for (const [id, source] of positive) {
    write(id + '.atto', source);
    const out = id + '.wasm';
    const r = invoke([id + '.atto', '-o', out]);
    assert.equal(r.status, 0, id + ': ' + r.stderr);
    assert.equal(r.stdout, '', id);
    const bytes = readFileSync(join(cwd, out));
    assert.ok(WebAssembly.validate(bytes), id);
    const module = new WebAssembly.Module(bytes);
    assert.deepEqual(WebAssembly.Module.imports(module), [], id);
    const instance = new WebAssembly.Instance(module, {});
    assert.equal(instance.exports.main(), 42n, id);
    passed++;
  }
  for (const [id, source] of negative) {
    const file = id + '.atto', out = id + '.wasm';
    write(file, source);
    for (const args of [[file, '-o', out], ['--check', file]]) {
      const r = invoke(args);
      assert.equal(r.status, 1, id + ': ' + r.stderr);
      assert.match(r.stderr, /\bE_UNSAFE_DOMAIN\b/, id);
      assert.equal(r.stdout, '', id + ': rejected source streamed bytes');
      assert.equal(existsSync(join(cwd, out)), false, id + ': rejected source emitted artifact');
      denied++;
    }
  }
  console.log(JSON.stringify({
    scope: 'rust_compiler_wasm_guest_denial_not_native_unsafe',
    compiler_sha256: createHash('sha256').update(readFileSync(attoc)).digest('hex'),
    safe_guests_executed: passed, unsafe_mode_denials: denied
  }));
} finally { rmSync(cwd, { recursive: true, force: true }); }

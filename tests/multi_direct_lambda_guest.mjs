// Compile real .atto source with native attoc and execute generated guest Wasm.
// This is deliberately not an interpreter, JS equivalent or expected-only test.
import assert from 'node:assert/strict';
import { readFile, mkdtemp, rm, stat } from 'node:fs/promises';
import { spawnSync } from 'node:child_process';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';

const attoc = process.env.ATTOC
  || fileURLToPath(new URL('../target/debug/attoc', import.meta.url));
const root = new URL('./fixtures/', import.meta.url);
const cases = [
  { path: 'pass/lambda-direct-three-binds.atto', result: 42n },
  { path: 'pass/lambda-direct-sibling-captures.atto', result: 42n },
  { path: 'pass/lambda-direct-sibling-shadow.atto', result: 42n },
  { path: 'fail/lambda-direct-two-binds.atto', reject: 'E_UNSUPPORTED' },
  { path: 'fail/lambda-direct-sibling-call.atto', reject: 'E_UNSUPPORTED' },
  { path: 'fail/lambda-direct-forward-call.atto', reject: 'E_UNSUPPORTED' },
  { path: 'fail/lambda-direct-sibling-arity.atto', reject: 'E_ARITY' },
];
// Also use the actual wasm32-built Rust compiler: compile the identical
// Atto bytes inside Wasm and compare the guest bytes to native attoc.
const compilerBytes = await readFile(
  new URL('../target/wasm32-unknown-unknown/release/atto_compiler.wasm', import.meta.url)
);
assert.ok(WebAssembly.validate(compilerBytes), 'real browser compiler must be Wasm');
const compilerModule = new WebAssembly.Module(compilerBytes);
assert.deepEqual(WebAssembly.Module.imports(compilerModule), [],
  'browser compiler must have zero ambient imports');
const compiler = new WebAssembly.Instance(compilerModule, {}).exports;
assert.equal(compiler.abi_version(), 2, 'pinned compiler ABI v2');
const encoder = new TextEncoder();
const decoder = new TextDecoder('utf8', { fatal: true });
function browserCompile(sourceText) {
  const input = encoder.encode(sourceText);
  assert.ok(input.length <= compiler.input_capacity(), 'browser input budget');
  const memory = new Uint8Array(compiler.memory.buffer);
  const inputPtr = compiler.input_ptr();
  assert.ok(inputPtr >= 0 && inputPtr + input.length <= memory.length,
    'browser compiler input bounds');
  memory.set(input, inputPtr);
  const status = compiler.compile(input.length);
  if (status !== 0) {
    assert.equal(compiler.result_length(), 0, 'failed browser compile produced guest bytes');
    const at = compiler.error_ptr();
    const mem = new Uint8Array(compiler.memory.buffer);
    assert.ok(at >= 0 && at < mem.length);
    let end = at;
    while (end < mem.length && end < at + 512 && mem[end] !== 0) end++;
    return { ok: false, diagnostic: decoder.decode(mem.subarray(at, end)) };
  }
  const at = compiler.result_ptr(), len = compiler.result_length();
  const mem = new Uint8Array(compiler.memory.buffer);
  assert.ok(len >= 8 && len <= (1 << 20) && at >= 0 && at + len <= mem.length,
    'browser compiler output bounds');
  const guest = mem.slice(at, at + len);
  assert.ok(WebAssembly.validate(guest));
  return { ok: true, guest };
}

const dir = await mkdtemp(join(tmpdir(), 'atto-multi-direct-'));
try {
  for (const [idx, row] of cases.entries()) {
    const source = fileURLToPath(new URL(row.path, root));
    const text = await readFile(source, 'utf8');
    const browser = browserCompile(text);
    const out = join(dir, idx + '.wasm');
    const cmd = spawnSync(attoc, [source, '-o', out], {
      encoding: 'utf8', timeout: 30_000, maxBuffer: 1 << 20,
    });
    assert.equal(cmd.error, undefined, row.path + ': native compiler process');
    if (row.reject) {
      assert.equal(browser.ok, false, row.path + ': unsupported browser guest was admitted');
      assert.ok(browser.diagnostic.includes(row.reject),
        row.path + ': browser diagnostic mismatch: ' + browser.diagnostic);
      assert.notEqual(cmd.status, 0, row.path + ': invalid guest was admitted');
      assert.ok(cmd.stderr.includes(row.reject), row.path + ': wrong diagnostic: ' + cmd.stderr);
      await assert.rejects(stat(out), { code: 'ENOENT' }, row.path + ': failed compile wrote guest');
      continue;
    }
    assert.equal(cmd.status, 0, row.path + ': ' + cmd.stderr);
    const bytes = await readFile(out);
    assert.equal(browser.ok, true, row.path + ': browser compiler rejected: ' + browser.diagnostic);
    assert.deepEqual(Buffer.from(browser.guest), bytes,
      row.path + ': native/browser compiler must emit identical guest Wasm');
    assert.ok(WebAssembly.validate(bytes), row.path + ': invalid Wasm bytes');
    const mod = await WebAssembly.compile(bytes);
    assert.deepEqual(WebAssembly.Module.imports(mod), [], row.path + ': unexpected host imports');
    const guest = await WebAssembly.instantiate(mod, {});
    assert.equal(guest.exports.main(), row.result, row.path + ': wrong guest return value');
    assert.equal(guest.exports.panic_code.value, 0, row.path + ': guest panic');
    console.log('PASS real multi-lambda guest', row.path, String(row.result));
  }
  // A rejected source must not poison the stateful browser compiler.
  const recovery = browserCompile('pub fnc main(): int { return 42; }');
  assert.equal(recovery.ok, true, 'browser compiler recovery after rejection');
  assert.equal(new WebAssembly.Instance(new WebAssembly.Module(recovery.guest), {})
    .exports.main(), 42n, 'recovery guest must execute');
} finally {
  await rm(dir, { recursive: true, force: true });
}

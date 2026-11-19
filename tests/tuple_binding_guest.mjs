// Executed guest-Wasm contract for bounded Copy-scalar Tuple lowering.
// Run: node tests/tuple_binding_guest.mjs
import { readFileSync, mkdtempSync, rmSync } from 'node:fs';
import { join, resolve } from 'node:path';
import { tmpdir } from 'node:os';
import { spawnSync } from 'node:child_process';
import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';

const cwd = resolve(import.meta.dirname, '..');
const fixture = (file) => join(cwd, 'tests', 'fixtures', 'tuple_binding', file);
const cargo = spawnSync('cargo', ['build', '--offline', '--bin', 'attoc'], {
  cwd, encoding: 'utf8', timeout: 180000,
});
if (cargo.error || cargo.status !== 0) {
  throw new Error('cargo build did not pass: ' + String(cargo.error ?? cargo.stderr));
}
const compiler = join(cwd, 'target', 'debug', process.platform === 'win32' ? 'attoc.exe' : 'attoc');
const dir = mkdtempSync(join(tmpdir(), 'atto-tuple-guest-'));

function runCompiler(input, output) {
  const args = [fixture(input)];
  if (output) args.push('-o', output);
  const result = spawnSync(compiler, args, { cwd, encoding: 'utf8', timeout: 30000 });
  if (result.error) throw result.error;
  return result;
}
async function positive(input, expected, printed = []) {
  const output = join(dir, input + '.wasm');
  const compiled = runCompiler(input, output);
  assert.equal(compiled.status, 0, input + ': ' + compiled.stderr);
  const bytes = readFileSync(output);
  assert.equal(bytes.subarray(0,4).toString('hex'), '0061736d');
  const printCalls = [];
  const { instance } = await WebAssembly.instantiate(bytes, {
    atto: { print_i64(value) { printCalls.push(value); } },
  });
  assert.deepEqual(printCalls, printed, input + ': tuple initializer must evaluate exactly once');
  assert.equal(instance.exports.main(), BigInt(expected), input + ': guest exit value mismatch');
  const second = join(dir, input + '.second.wasm');
  const again = runCompiler(input, second);
  assert.equal(again.status, 0, input + ': repeat failed: ' + again.stderr);
  assert.equal(
    createHash('sha256').update(bytes).digest('hex'),
    createHash('sha256').update(readFileSync(second)).digest('hex'),
    input + ': native compiler emitted non-deterministic bytes',
  );
}
function negative(input, diagnostic) {
  const output = join(dir, input + '.wasm');
  const result = runCompiler(input, output);
  assert.equal(result.status, 1, input + ' unexpectedly compiled: ' + result.stderr);
  assert.match(result.stderr, new RegExp(diagnostic), input + ': wrong error');
}
try {
  await positive('let_root_guest.atto', 38);
  await positive('val_root_guest.atto', 60);
  await positive('const_root_guest.atto', 8);
  await positive('once_only_guest.atto', 6, [99n]);
  negative('arity_fail.atto', 'E_ARITY');
  negative('runtime_const_fail.atto', 'E_CONST');
  negative('scalar_const_mut_fail.atto', 'E_CONST_MUT_TARGET');
  console.log('Tuple guest: 4/4 executed, 3/3 negative diagnostics, 4/4 deterministic-byte matches, once-only effect verified.');
} finally {
  rmSync(dir, { recursive: true, force: true });
}

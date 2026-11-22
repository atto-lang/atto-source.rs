import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtempSync, writeFileSync, chmodSync, rmSync } from 'node:fs';
import { join } from 'node:path';
import { tmpdir } from 'node:os';
import { nativeAttocBytes } from './native-attoc-strict.mjs';

const source = '/tmp/atto-strict-native-fixture.atto';
const failure = { ok:false, code:'E_UNBOUND', message:'E_UNBOUND: unknown value', line:2, col:3 };
const wasm = Buffer.from('0061736d01000000','hex');
function withFakeAttoc(body, run) {
  const dir = mkdtempSync(join(tmpdir(), 'atto-native-cli-test-'));
  const exe = join(dir, 'attoc');
  writeFileSync(exe, '#!' + process.execPath + '\n' + body + '\n');
  chmodSync(exe, 0o700);
  try { return run(exe); } finally { rmSync(dir, { recursive:true, force:true }); }
}
const normalReject = 'process.stderr.write(process.argv[2] + ": 2:3: E_UNBOUND: unknown value\\n"); process.exit(1);';

test('accepts genuine byte-identical WebAssembly success', () => withFakeAttoc(
  `process.stdout.write(Buffer.from('${wasm.toString('hex')}','hex'));`,
  exe => assert.deepEqual(Buffer.from(nativeAttocBytes(exe, source)), wasm)
));
test('accepts exact native/Wasm diagnostic with compiler rejection status 1', () => withFakeAttoc(
  normalReject, exe => assert.equal(nativeAttocBytes(exe, source, failure), null)
));
test('rejects wrong native source position and diagnostic message', () => withFakeAttoc(
  'process.stderr.write(process.argv[2] + ": 3:3: E_UNBOUND: unknown value\\n"); process.exit(1);',
  exe => assert.throws(() => nativeAttocBytes(exe, source, failure), /not byte-identical/)
));
test('rejects missing executable instead of passing expected negative case', () => {
  assert.throws(() => nativeAttocBytes('/tmp/does-not-exist-atto-native-attoc', source, failure));
});
test('rejects compiler CLI usage error exit 2 even with E_ diagnostic', () => withFakeAttoc(
  'process.stderr.write(process.argv[2] + ": 2:3: E_UNBOUND: unknown value\\n"); process.exit(2);',
  exe => assert.throws(() => nativeAttocBytes(exe, source, failure), /unexpected native attoc exit/)
));
test('rejects crashed compiler by signal', () => withFakeAttoc(
  'process.kill(process.pid, "SIGTERM");',
  exe => assert.throws(() => nativeAttocBytes(exe, source, failure), /terminated by signal/)
));
test('rejects timed-out compiler instead of passing expected negative case', () => withFakeAttoc(
  'setTimeout(() => {}, 30000);',
  exe => assert.throws(() => nativeAttocBytes(exe, source, failure, {timeoutMs:100}), /ETIMEDOUT|SIGTERM|timed out/)
));
test('rejects compiler failure without stderr diagnostics', () => withFakeAttoc(
  'process.exit(1);',
  exe => assert.throws(() => nativeAttocBytes(exe, source, failure), /not byte-identical/)
));
test('rejects any executable output when compiler claims rejection', () => withFakeAttoc(
  `process.stdout.write('bad wasm'); ${normalReject}`,
  exe => assert.throws(() => nativeAttocBytes(exe, source, failure), /rejected native source emitted bytes/)
));
test('rejects invalid Wasm on compiler success', () => withFakeAttoc(
  'process.stdout.write("not a wasm module");',
  exe => assert.throws(() => nativeAttocBytes(exe, source), /invalid Wasm/)
));
test('rejects malformed stderr when no Wasm diagnostic is provided', () => withFakeAttoc(
  'process.stderr.write("not a compiler diagnostic\\n"); process.exit(1);',
  exe => assert.throws(() => nativeAttocBytes(exe, source), /identify its input source/)
));
test('accepts compiler-formatted E_ rejection without explicit Wasm diagnostic', () => withFakeAttoc(
  normalReject, exe => assert.equal(nativeAttocBytes(exe, source), null)
));
test('rejects native success when Wasm compiler rejected source', () => withFakeAttoc(
  `process.stdout.write(Buffer.from('${wasm.toString('hex')}','hex'));`,
  exe => assert.throws(() => nativeAttocBytes(exe, source, failure), /accepted rejected Wasm source/)
));

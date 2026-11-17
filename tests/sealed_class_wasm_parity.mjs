// Exact compiled-host comparison for sealed classes. Only source-local class
// composition is exercised; this is NOT dynamic subtype dispatch or AOT
// cross-module subclass linking.
import assert from 'node:assert/strict';
import {readFileSync, mkdtempSync, writeFileSync, rmSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {join, resolve} from 'node:path';
import {spawnSync} from 'node:child_process';

const compilerPath = process.env.ATTO_COMPILER_WASM ||
  'target/wasm32-unknown-unknown/release/atto_compiler.wasm';
const nativePath = resolve(process.env.ATTOC || 'target/release/attoc');
const browserCompiler = new WebAssembly.Module(readFileSync(compilerPath));
assert.deepEqual(WebAssembly.Module.imports(browserCompiler), [],
  'browser compiler must not depend on unverified host imports');
const {exports: api} = new WebAssembly.Instance(browserCompiler, {});
assert.equal(api.abi_version(), 2);
const encoder = new TextEncoder();
const decoder = new TextDecoder('utf-8', {fatal: true});
const tmp = mkdtempSync(join(tmpdir(), 'atto-sealed-parity-'));
const main = 'pub fnc main(): int { return 42; }';
const cases = [
  ['final-leaf', 'define sealed class Root permits Leaf as pub answer(): int {return 42;} end define final class Leaf extends Root as end pub fnc main(): int {val obj = new Leaf(); return obj.answer();}', null],
  ['nonsealed-descendant', 'define sealed class Root permits Branch as end define non-sealed class Branch extends Root as end define class Child extends Branch as end ' + main, null],
  ['recursive-closure', 'define sealed class Root permits Middle as end define sealed class Middle extends Root permits Leaf as end define final class Leaf extends Middle as end ' + main, null],
  ['authorized-diamond', 'define sealed class A permits Leaf as pub left(): int {return 20;} end define sealed class B permits Leaf as pub right(): int {return 22;} end define final class Leaf extends A, B as end pub fnc main(): int {val x = new Leaf(); return x.left() + x.right();}', null],
  ['public-export', 'export pub class Root; define sealed class Root permits Leaf as end define final class Leaf extends Root as end ' + main, null],
  ['unrelated-literal', 'define final class Closed as end pub fnc main(): int {val text = "define sealed class Bad permits Evil as end"; return 42;}', null],
  ['unlisted-child', 'define sealed class Root permits Good as end define final class Good extends Root as end define final class Evil extends Root as end ' + main, 'E_SEALED'],
  ['unknown-child', 'define sealed class Root permits Missing as end ' + main, 'E_SEALED'],
  ['wrong-kind-permit', 'define interface I as ping(): int; end define sealed class Root permits I as end ' + main, 'E_SEALED'],
  ['final-extended', 'define final class Closed as end define class Child extends Closed as end ' + main, 'E_FINAL'],
  ['orphan-nonsealed', 'define non-sealed class Child as end ' + main, 'E_SEALED'],
  ['space-in-modifier', 'define sealed class Root permits Child as end define non - sealed class Child extends Root as end ' + main, 'E_SEALED'],
  ['duplicate-permits', 'define sealed class Root permits Child, Child as end define final class Child extends Root as end ' + main, 'E_SEALED'],
  ['unmarked-child', 'define sealed class Root permits Child as end define class Child extends Root as end ' + main, 'E_SEALED'],
  ['unauthorized-diamond', 'define sealed class A permits Leaf as end define sealed class B permits Other as end define final class Leaf extends A, B as end define final class Other extends B as end ' + main, 'E_SEALED'],
  ['trait-extends', 'define class Root as end define trait T extends Root as pub ping(): int {return 1;} end ' + main, 'E_TYPE'],
  ['interface-extends', 'define class Root as end define interface I extends Root as ping(): int; end ' + main, 'E_TYPE'],
  ['trait-implements', 'define interface I as ping(): int; end define trait T implements I as pub ping(): int {return 1;} end ' + main, 'E_TYPE'],
  ['bad-override', 'define sealed class Root permits Leaf as pub f(): int {return 1;} end define final class Leaf extends Root as pub override f(): bool {return true;} end ' + main, 'E_SIGNATURE'],
  ['sealed-trait', 'define sealed trait T permits Child as end ' + main, 'E_SEALED']
];
function wasmCompile(source) {
  const bytes = encoder.encode(source);
  assert.ok(bytes.length <= api.input_capacity(), 'input capacity');
  new Uint8Array(api.memory.buffer, api.input_ptr(), bytes.length).set(bytes);
  const code = api.compile(bytes.length);
  if (code === 0) {
    const emitted = new Uint8Array(api.memory.buffer, api.result_ptr(), api.result_length()).slice();
    assert.ok(WebAssembly.validate(emitted), 'compiler must emit valid guest Wasm');
    return {ok:true, bytes:Buffer.from(emitted)};
  }
  assert.equal(code, 1, 'compiler ABI should return a stable failure');
  assert.equal(api.result_length(), 0, 'rejected compile cannot expose stale guest Wasm');
  const memory = new Uint8Array(api.memory.buffer);
  const start = api.error_ptr();
  let end = start;
  while (end < memory.length && memory[end]) end++;
  const message = decoder.decode(memory.subarray(start, end));
  return {ok:false, code:message.split(':')[0],
    line:api.error_line_number(), col:api.error_column_number()};
}
try {
  let passes = 0, rejections = 0, guests = 0;
  for (const [id, code, error] of cases) {
    const file = join(tmp, id + '.atto');
    writeFileSync(file, code, 'utf8');
    const native = spawnSync(nativePath, [file], {encoding: null, maxBuffer: 3 * 1024 * 1024});
    if (native.error) throw native.error;
    const browser = wasmCompile(code);
    if (error) {
      assert.equal(native.status, 1, id + ' native must fail');
      assert.match(native.stderr.toString(), new RegExp('\\b' + error + ':'), id + ' native diagnostic');
      assert.equal(native.stdout.length, 0, id + ' native must produce no guest artifact');
      assert.equal(browser.ok, false, id + ' browser must reject');
      assert.equal(browser.code, error, id + ' browser diagnostic parity');
      assert.ok(browser.line > 0 && browser.col > 0, id + ' diagnostic position');
      rejections++;
    } else {
      assert.equal(native.status, 0, id + ' native: ' + native.stderr.toString());
      assert.equal(browser.ok, true, id + ' browser compile');
      assert.deepEqual(browser.bytes, native.stdout, id + ' cross-host byte parity');
      const mod = new WebAssembly.Module(browser.bytes);
      assert.deepEqual(WebAssembly.Module.imports(mod), [], id + ' no unverified guest imports');
      const guest = new WebAssembly.Instance(mod, {});
      assert.equal(guest.exports.main(), 42n, id + ' compiled guest result');
      passes++; guests++;
    }
    // The same browser compiler instance is reused across different error and
    // success shapes to catch stale results and cross-request state pollution.
  }
  const recovery = wasmCompile(main);
  assert.equal(recovery.ok, true, 'browser compiler must recover after rejection');
  assert.equal(new WebAssembly.Instance(new WebAssembly.Module(recovery.bytes),{}).exports.main(),42n);
  console.log('PASS_SEALED_CROSS_HOST_PARITY ' + JSON.stringify({
    cases:cases.length, accepted:passes, rejected:rejections, guests,
    nativeVsBrowserBytes:true, browserAbi:true, postErrorRecovery:true
  }));
} finally {
  rmSync(tmp, {recursive:true, force:true});
}

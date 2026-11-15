// Executes the actual wasm32-built Atto compiler AND its generated M8 guest
// without interpreter shims. Also compares to the native Rust compiler binary.
import assert from 'node:assert/strict';
import {readFileSync, mkdtempSync, rmSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {join} from 'node:path';
import {execFileSync} from 'node:child_process';
const path = process.argv[2] || 'target/wasm32-unknown-unknown/release/atto_compiler.wasm';
const compilerBytes = readFileSync(path);
const compilerModule = new WebAssembly.Module(compilerBytes);
assert.deepEqual(WebAssembly.Module.imports(compilerModule), []);
const {exports: comp} = new WebAssembly.Instance(compilerModule);
assert.equal(comp.abi_version(), 2);
const text = new TextEncoder();
const source = readFileSync('tests/fixtures/m8/cooperative_sum.atto', 'utf8');
function compile(input) {
  const src = text.encode(input);
  assert.ok(src.length <= comp.input_capacity());
  new Uint8Array(comp.memory.buffer, comp.input_ptr(), src.length).set(src);
  const rc = comp.compile(src.length);
  if (rc !== 0) {
    assert.equal(comp.result_length(), 0);
    const mem = new Uint8Array(comp.memory.buffer);
    let end = comp.error_ptr();
    while (mem[end]) end++;
    return {err: new TextDecoder().decode(mem.subarray(comp.error_ptr(),end))};
  }
  return {bytes: new Uint8Array(comp.memory.buffer, comp.result_ptr(), comp.result_length()).slice()};
}
const one = compile(source);
assert.ok(one.bytes);
assert.ok(WebAssembly.validate(one.bytes), 'emitted guest not valid Wasm');
assert.deepEqual(compile(source).bytes, one.bytes, 'nondeterministic guest bytes');
if (process.env.ATTOC) {
  const tmp = mkdtempSync(join(tmpdir(), 'm8-'));
  try {
    const src = join(tmp, 'fixture.atto');
    const fs = await import('node:fs');
    fs.writeFileSync(src, source);
    const native = execFileSync(process.env.ATTOC, [src]);
    assert.deepEqual(one.bytes, native, 'host Rust and wasm32 compiler differ');
  } finally {rmSync(tmp,{recursive:true,force:true});}
}
const wasm = new WebAssembly.Module(one.bytes);
assert.deepEqual(WebAssembly.Module.imports(wasm), []);
assert.deepEqual(WebAssembly.Module.exports(wasm).map(x=>x.name).sort(),
  ['m8_close','m8_index','m8_new','m8_pc','m8_result','m8_resume'].sort());
assert.equal(WebAssembly.Module.customSections(wasm,'atto.m8.cooperate').length, 1);
const {exports: a} = new WebAssembly.Instance(wasm);
const h = a.m8_new(100_000n);
assert.equal(h,1n);
assert.equal(a.m8_pc(h),0);
assert.throws(()=>a.m8_new(7n),WebAssembly.RuntimeError,'cannot start second in-flight frame');
assert.throws(()=>a.m8_resume(h,0),WebAssembly.RuntimeError);
assert.throws(()=>a.m8_resume(h,65537),WebAssembly.RuntimeError);
assert.throws(()=>a.m8_result(h),WebAssembly.RuntimeError,'result before done');
let yields=0, lastIndex=0n, rc;
do {
  rc=a.m8_resume(h,4);
  assert.ok(rc===0||rc===1);
  const i=a.m8_index(h);
  assert.ok(i>=lastIndex && i-lastIndex<=4n,'duplicated/skipped loop slice');
  lastIndex=i;
  if(rc===1) {yields++;assert.equal(a.m8_pc(h),1);}
} while(rc===1 && yields<100_000);
assert.equal(rc,0);
assert.ok(yields>=24_999,'expected reduction-budget yields');
assert.equal(a.m8_index(h),100_000n);
assert.equal(a.m8_result(h),4_999_950_000n);
assert.equal(a.m8_pc(h),4);
assert.equal(a.m8_resume(h,1),0,'DONE must be idempotent');
a.m8_close(h);
assert.throws(()=>a.m8_resume(h,1),WebAssembly.RuntimeError);
assert.throws(()=>a.m8_result(h),WebAssembly.RuntimeError);
assert.throws(()=>a.m8_close(h),WebAssembly.RuntimeError);
const newHandle = a.m8_new(10n);
assert.ok(newHandle>h);
assert.throws(()=>a.m8_resume(h,1),WebAssembly.RuntimeError,'stale generation');
assert.equal(a.m8_resume(newHandle,65536),0);
assert.equal(a.m8_result(newHandle),45n);
a.m8_close(newHandle);
console.log('M8_CANONICAL_RUST_WASM_COMPILED_GUEST=PASS');
console.log('M8_WASM_COOPERATIVE_YIELDS='+yields);
console.log('M8_WASM_GENERATION_FENCE=PASS');
console.log('M8_WASM_EXACT_SUM=4999950000');
const b = new WebAssembly.Instance(wasm).exports;
const bh=b.m8_new(5n);
assert.equal(b.m8_resume(bh,1),1);
assert.equal(b.m8_pc(bh),1);
assert.equal(b.m8_resume(bh,65536),0);
assert.equal(b.m8_result(bh),10n);
b.m8_close(bh);
// This admitted profile checks positive overflow BEFORE mutating state.
const overflow = source.replace('let total: int = 0;','let total: int = 9223372036854775807;')
  .replace('let i: int = 0;','let i: int = 1;');
const ov=compile(overflow);
assert.ok(ov.bytes && WebAssembly.validate(ov.bytes));
const o=new WebAssembly.Instance(new WebAssembly.Module(ov.bytes)).exports;
const oh=o.m8_new(3n);
assert.throws(()=>o.m8_resume(oh,1),WebAssembly.RuntimeError,'overflow must trap');
assert.throws(()=>o.m8_resume(oh,1),WebAssembly.RuntimeError,'trapped frame must not resume');
console.log('M8_WASM_I64_OVERFLOW_FAILS_CLOSED=PASS');
for (const bad of [
  source.replace('rt cooperate;','rt borrow total; rt cooperate;'),
  source.replace('rt cooperate;','critical-write total; rt cooperate;'),
  source.replace('rt cooperate;','await job; rt cooperate;'),
  source.replace('i = i + 1;','i = i + 2;'),
]) {
  const outcome=compile(bad);
  assert.ok(outcome.err?.includes('E_UNSUPPORTED'), 'unsafe extension compiled');
}
console.log('M8_WASM_UNSUPPORTED_EFFECTS_REJECTED=PASS');
console.log('M8_SCOPE=ONE_FRAME_PER_WASM_INSTANCE_NOT_FULL_ACTOR_MAILBOX_SELECT_OR_HARD_RT');

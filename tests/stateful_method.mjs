// Exact native/wasm32 source compiler differential + real guest semantics.
// No mocks and no synthetic cleanup-witness state substitution.
import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
import {spawnSync} from 'node:child_process';
const sourceFile=new URL('./fixtures/stateful/counter.atto',import.meta.url);
const source=readFileSync(sourceFile,'utf8');
const native=spawnSync(process.env.ATTOC||'target/release/attoc',
  ['--stateful-method-preview',sourceFile.pathname],{encoding:null});
assert.equal(native.status,0,String(native.stderr));
const bytes=new Uint8Array(native.stdout);
assert.ok(WebAssembly.validate(bytes));
const compiler=new WebAssembly.Instance(new WebAssembly.Module(readFileSync(
  process.argv[2]||'target/wasm32-unknown-unknown/release/atto_compiler.wasm')),{})
  .exports;
const data=new TextEncoder().encode(source);
new Uint8Array(compiler.memory.buffer,compiler.input_ptr(),data.length).set(data);
assert.equal(compiler.compile_stateful_method_preview(data.length),0);
const browser=Uint8Array.from(new Uint8Array(compiler.memory.buffer,
  compiler.result_ptr(),compiler.result_length()));
assert.deepEqual([...browser],[...bytes],
  'native/wasm32 must emit byte-identical Wasm from the actual source method');
const mod=new WebAssembly.Module(bytes);
assert.deepEqual(WebAssembly.Module.imports(mod),[]);
const a=new WebAssembly.Instance(mod,{}).exports;
const b=new WebAssembly.Instance(mod,{}).exports;
const inc=a['atto.state.Counter.increment.1'];
const state=a['atto.state.Counter.count'];
assert.equal(state(),17n,'initial state comes from actual actor initializer');
assert.equal(inc(25n,0),42n,'actual source state method s.count=s.count+delta');
assert.equal(state(),42n);
assert.equal(a.acquire_count(),1);
assert.equal(a.release_count(),1);
assert.equal(a.lease_active(),0);
assert.throws(()=>inc(9223372036854775807n,0),WebAssembly.RuntimeError);
assert.equal(a.panic_code.value,1);
assert.equal(state(),42n,'overflow does not commit partial state');
assert.equal(a.acquire_count(),2);
assert.equal(a.release_count(),2);
assert.equal(a.lease_active(),0);
assert.equal(inc(99n,1),-1n,'cooperative cancel has no effect');
assert.equal(state(),42n);
assert.equal(a.release_count(),3);
assert.equal(a.acquire_count(),3);
assert.equal(b['atto.state.Counter.count'](),17n,'second actor is isolated');
assert.equal(b['atto.state.Counter.increment.1'](1n,0),18n);
assert.equal(a.close_witness(),1);
assert.equal(a.close_witness(),0);
assert.throws(()=>inc(1n,0),WebAssembly.RuntimeError);
assert.equal(a.panic_code.value,7);
assert.equal(a.release_count(),3,'closed method must not leak/double release');
assert.equal(a.acquire_count(),3);
console.log('PASS real compiled stateful Counter.increment: native/wasm32 parity, source initial 17, checked write 42, overflow rollback, cancel, instance isolation and closed generation');

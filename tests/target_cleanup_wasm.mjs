// Real native-compiler / wasm32-compiler -> Wasm lease-cleanup witness.
// This is a *synthetic* checked state update, not executed Atto source body.
import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
import {spawnSync} from 'node:child_process';

const source=readFileSync(new URL('./fixtures/cleanup/witness.atto',import.meta.url),'utf8');
const hostCompiler=process.env.ATTOC || 'target/release/attoc';
const wasmCompiler=process.argv[2] ||
  'target/wasm32-unknown-unknown/release/atto_compiler.wasm';
const compiler=new WebAssembly.Instance(new WebAssembly.Module(readFileSync(wasmCompiler)),{}).exports;
assert.equal(compiler.abi_version(),2);
assert.equal(typeof compiler.compile_cleanup_wasm_witness,'function');
const data=new TextEncoder().encode(source);
function compile_browser(mode) {
  assert.ok(data.length<=compiler.input_capacity());
  new Uint8Array(compiler.memory.buffer,compiler.input_ptr(),data.length).set(data);
  const rc=compiler[mode](data.length);
  if(rc===0)return {ok:true,bytes:Uint8Array.from(new Uint8Array(
    compiler.memory.buffer,compiler.result_ptr(),compiler.result_length()))};
  assert.equal(rc,1);
  let p=compiler.error_ptr(),end=p,mem=new Uint8Array(compiler.memory.buffer);
  while(mem[end]!==0)end++;
  return {ok:false,error:new TextDecoder().decode(mem.subarray(p,end))};
}
const n=spawnSync(hostCompiler,['--cleanup-wasm-witness',
  new URL('./fixtures/cleanup/witness.atto',import.meta.url).pathname],{encoding:null});
assert.equal(n.status,0,String(n.stderr));
const native=new Uint8Array(n.stdout);
const browser=compile_browser('compile_cleanup_wasm_witness');
assert.ok(browser.ok,browser.error);
assert.deepEqual([...native],[...browser.bytes],
  'native and wasm32 compiler must emit byte-identical audited cleanup Wasm');
assert.ok(WebAssembly.validate(native),'the cleanup witness must execute in the real Wasm VM');
assert.equal(WebAssembly.Module.imports(new WebAssembly.Module(native)).length,0);
assert.match(compile_browser('compile').error,/^E_UNSUPPORTED:/);
assert.match(compile_browser('compile_receiver_instance_preview').error,/^E_UNSUPPORTED:/);

const witness=()=>new WebAssembly.Instance(new WebAssembly.Module(native),{}).exports;
const a=witness(),b=witness();
assert.equal(a.lease_active(),0);
assert.equal(a.acquire_count(),0);
assert.equal(a.release_count(),0);
assert.equal(a.state_value(),41n);
assert.equal(a.run_cleanup_witness(1n,0),42n);
assert.equal(a.lease_active(),0);
assert.equal(a.acquire_count(),1);
assert.equal(a.release_count(),1);
assert.equal(a.state_value(),42n);
assert.throws(()=>a.run_cleanup_witness(9223372036854775807n,0),
  WebAssembly.RuntimeError);
assert.equal(a.panic_code.value,1,'actual checked overflow must set panic code 1');
assert.equal(a.lease_active(),0,'overflow edge must unlock before hard Wasm trap');
assert.equal(a.release_count(),2,'overflow edge must release exactly once');
assert.equal(a.acquire_count(),2);
assert.equal(a.state_value(),42n,'overflow must not commit partial mutation');
assert.equal(a.run_cleanup_witness(123n,1),-1n,'cooperative cancel is explicit');
assert.equal(a.lease_active(),0,'cancel edge must unlock before return');
assert.equal(a.release_count(),3);
assert.equal(a.acquire_count(),3);
assert.equal(a.state_value(),42n);
assert.equal(b.run_cleanup_witness(1n,0),42n,'separate instance has independent state');
assert.equal(b.release_count(),1);
assert.equal(a.close_witness(),1);
assert.equal(a.close_witness(),0,'close is terminal and idempotent');
assert.throws(()=>a.run_cleanup_witness(1n,0),WebAssembly.RuntimeError);
assert.equal(a.panic_code.value,7);
assert.equal(a.acquire_count(),3,'closed instance must refuse acquisition');
assert.equal(a.release_count(),3,'closed rejection must not double release');
assert.equal(a.lease_active(),0);

for(const mutation of [
  'declare actor A; mtd (A self) f(int n): int { return n; }',
  'declare actor A; mtd (A self) f(int n): int { return n + 1; }'
]) {
  assert.notEqual(mutation,source);
  const payload=new TextEncoder().encode(mutation);
  new Uint8Array(compiler.memory.buffer,compiler.input_ptr(),payload.length).set(payload);
  assert.equal(compiler.compile_cleanup_wasm_witness(payload.length),1,
    'no non-lease witness output from malformed receiver');
  assert.equal(compiler.result_length(),0);
}
console.log('PASS emitted cleanup witness: native/browser byte parity, real Wasm checked overflow, normal/panic/cancel exactly-once release, terminal close and fail-closed production');

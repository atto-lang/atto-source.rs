// Actual native/browser compiler -> executable actor-instance Wasm dispatch.
// No imports, each instance has private nominal owner, lifecycle and token.
import assert from 'node:assert/strict';
import {readFileSync,writeFileSync,mkdtempSync,rmSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {join} from 'node:path';
import {spawnSync} from 'node:child_process';
import {create_receiver_runtime} from '../runtime/receiver_instances.mjs';

const source=`
use namespace workers;
declare actor Worker;
declare actor Other;
mtd (Worker self) double(int n): int { return n * 2; }
mtd (Worker self) add(int n, int b): int { return n + b; }
mtd (Other self) double(int n): int { return n * 3; }
`;
const hostCompiler=process.env.ATTOC || 'target/release/attoc';
const browserCompiler=process.argv[2] || 'target/wasm32-unknown-unknown/release/atto_compiler.wasm';
const compiler=new WebAssembly.Instance(new WebAssembly.Module(readFileSync(browserCompiler)),{}).exports;
assert.equal(compiler.abi_version(),2);
assert.equal(typeof compiler.compile_receiver_instance_preview,'function');
const encoder=new TextEncoder(),decoder=new TextDecoder();
function compile_browser(src,preview=true) {
  const data=encoder.encode(src);
  assert.ok(data.length<=compiler.input_capacity());
  new Uint8Array(compiler.memory.buffer,compiler.input_ptr(),data.length).set(data);
  const rc=(preview?compiler.compile_receiver_instance_preview:compiler.compile)(data.length);
  if(rc===0) return {ok:true,bytes:Uint8Array.from(new Uint8Array(
    compiler.memory.buffer,compiler.result_ptr(),compiler.result_length()))};
  assert.equal(rc,1);
  let start=compiler.error_ptr(),end=start,buf=new Uint8Array(compiler.memory.buffer);
  while(buf[end]!==0)end++;
  return {ok:false,error:decoder.decode(buf.subarray(start,end))};
}
const temp=mkdtempSync(join(tmpdir(),'atto-receiver-instance-'));
let native;
try {
  const input=join(temp,'actor.atto');
  writeFileSync(input,source);
  const result=spawnSync(hostCompiler,['--receiver-instance-preview',input],{encoding:null});
  assert.equal(result.status,0,String(result.stderr));
  native=new Uint8Array(result.stdout);
} finally {rmSync(temp,{recursive:true,force:true});}
const browser=compile_browser(source);
assert.ok(browser.ok,browser.error);
assert.deepEqual([...native],[...browser.bytes],'native/browser must emit byte-identical actor-instance Wasm');
assert.ok(WebAssembly.validate(native));
assert.deepEqual(WebAssembly.Module.imports(new WebAssembly.Module(native)),[]);
assert.match(compile_browser(source,false).error,/^E_UNSUPPORTED:/);

// Test the guest ABI directly, including incorrect tokens/owner and terminal
// close. All checks must execute inside the generated guest, not JS mocks.
function guest() {return new WebAssembly.Instance(new WebAssembly.Module(native),{}).exports;}
{
  const raw=guest();
  assert.equal(raw['atto.bind.Worker'](0n),0);
  assert.throws(()=>raw['atto.receiver.Worker.double.1'](55n,21n),WebAssembly.RuntimeError);
  assert.equal(raw.panic_code.value,6);
  assert.equal(raw['atto.bind.Worker'](55n),1);
  assert.equal(raw['atto.bind.Worker'](55n),0,'double bind rejected');
  assert.equal(raw['atto.bind.Other'](55n),0,'cross-owner rebind rejected');
  assert.throws(()=>raw['atto.receiver.Worker.double.1'](54n,21n),WebAssembly.RuntimeError);
  assert.equal(raw.panic_code.value,6);
  assert.throws(()=>raw['atto.receiver.Other.double.1'](55n,21n),WebAssembly.RuntimeError);
  assert.equal(raw['atto.receiver.Worker.double.1'](55n,21n),42n);
  assert.equal(raw['atto.actor_close'](54n),0,'wrong token cannot close');
  assert.equal(raw['atto.actor_close'](55n),1);
  assert.equal(raw['atto.actor_close'](55n),0,'duplicate close rejected');
  assert.equal(raw['atto.bind.Worker'](55n),0,'closed instance must never rebind');
  assert.throws(()=>raw['atto.receiver.Worker.double.1'](55n,21n),WebAssembly.RuntimeError);
}

const notices=[];
const runtime=create_receiver_runtime(native,{on_control:n=>notices.push(n)});
const a=runtime.spawn_actor('tenant-A','Worker');
const b=runtime.spawn_actor('tenant-A','Worker');
const c=runtime.spawn_actor('tenant-B','Other');
assert.equal(runtime.invoke_method('tenant-A',a,'double',21n),42n);
assert.equal(runtime.invoke_method('tenant-A',b,'double',-4n),-8n);
assert.equal(runtime.invoke_method('tenant-B',c,'double',14n),42n);
assert.equal(runtime.invoke_method('tenant-A',b,'add',10n,32n),42n);
assert.throws(()=>runtime.invoke_method('tenant-B',a,'double',21n),/cross-tenant/);
assert.throws(()=>runtime.invoke_method('tenant-A',{},'double',21n),/forged/);
assert.throws(()=>runtime.invoke_method('tenant-A',c,'double',21n),/cross-tenant/);
assert.throws(()=>runtime.invoke_method('tenant-A',a,'missing',3n),/unknown receiver/);
assert.throws(()=>runtime.invoke_method('tenant-A',a,'double',21),/signed i64/);
assert.throws(()=>runtime.invoke_method('tenant-A',a,'double',1n<<63n),/signed i64/);
assert.equal(notices.length,0,'host rejections cannot forge supervisor notices');
assert.equal(runtime.close_actor('tenant-A',a),true);
assert.equal(runtime.close_actor('tenant-A',a),false);
assert.throws(()=>runtime.invoke_method('tenant-A',a,'double',21n),/closed/);
assert.throws(()=>runtime.spawn_actor('tenant-C','Unknown'),/unknown actor type/);

// A real guest checked overflow terminates only the affected actor instance,
// publishes one host CONTROL hook, and does not contaminate sibling instances.
const bad=runtime.spawn_actor('tenant-A','Worker');
assert.throws(()=>runtime.invoke_method('tenant-A',bad,'double',9223372036854775807n),
  WebAssembly.RuntimeError);
assert.equal(notices.length,1);
assert.equal(notices[0].kind,'PANIC');
assert.equal(notices[0].panic_code,1);
assert.equal(notices[0].tenant,'tenant-A');
assert.equal(notices[0].actor_type,'Worker');
assert.equal(notices[0].generation,1);
assert.throws(()=>runtime.invoke_method('tenant-A',bad,'double',1n),/closed/);
assert.equal(runtime.invoke_method('tenant-A',b,'double',21n),42n);
assert.equal(runtime.invoke_method('tenant-B',c,'double',14n),42n);
assert.equal(runtime.close_actor('tenant-A',b),true);
assert.equal(runtime.close_actor('tenant-B',c),true);
assert.equal(notices.length,1);

for(const invalid of [
 'declare actor A; mtd (mut self: A) change(int n): int { return n; }',
 'declare actor A; mtd (A self) state(int n): int { self.handle_state(critical-read |s| -> s.x); return n; }',
 'declare actor A; mtd (A self) loop(int n): int { while true { return n; } }',
 'declare actor A; mtd (A self) divide(int n): int { return n / 2; }'
]) {
  const failed=compile_browser(invalid);
  assert.equal(failed.ok,false,invalid);
  assert.match(failed.error,/^E_/);
}
console.log('PASS actor instance native/browser byte parity, bound Wasm dispatch, owner/token checks, terminal close, tenant isolation and checked-panic hook');

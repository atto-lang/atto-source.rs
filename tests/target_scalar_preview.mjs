// Native and browser compiler must produce identical executable stateless
// receiver Wasm; regular compile() must remain fail-closed for all mtd source.
import assert from 'node:assert/strict';
import {readFileSync, writeFileSync, mkdtempSync, rmSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {join} from 'node:path';
import {spawnSync} from 'node:child_process';

const source=`
use namespace workers;
declare actor Worker;
mtd (Worker self) double(int n): int { return n * 2; }
mtd (Worker self) add(int n, int b): int { return n + b; }
mtd (Worker self) subtract(int n): int { return n - 1; }
mtd (Worker self) minus(int n): int { return -n; }
mtd (Worker self) minimum(): int { return -9223372036854775808; }
`;
const hostCompiler=process.env.ATTOC || 'target/release/attoc';
const browserCompiler=process.argv[2] || 'target/wasm32-unknown-unknown/release/atto_compiler.wasm';
const browser=new WebAssembly.Instance(new WebAssembly.Module(readFileSync(browserCompiler)),{});
const api=browser.exports;
assert.equal(api.abi_version(),2);
assert.equal(typeof api.compile_scalar_preview,'function');
const enc=new TextEncoder(), dec=new TextDecoder();
function compileBrowser(src,preview=true) {
  const input=enc.encode(src);
  assert.ok(input.length<=api.input_capacity());
  new Uint8Array(api.memory.buffer,api.input_ptr(),input.length).set(input);
  const rc=(preview?api.compile_scalar_preview:api.compile)(input.length);
  if(rc===0) return {ok:true,bytes:new Uint8Array(api.memory.buffer,api.result_ptr(),api.result_length()).slice()};
  assert.equal(rc,1);
  assert.equal(api.result_length(),0);
  const mem=new Uint8Array(api.memory.buffer);
  const start=api.error_ptr();let stop=start;
  while(mem[stop]!==0)stop++;
  return {ok:false,error:dec.decode(mem.subarray(start,stop))};
}
const tmp=mkdtempSync(join(tmpdir(),'atto-scalar-preview-'));
let native;
try {
  const input=join(tmp,'receiver.atto');
  writeFileSync(input,source);
  const proc=spawnSync(hostCompiler,['--scalar-preview',input],{encoding:null});
  assert.equal(proc.status,0,proc.stderr.toString());
  native=new Uint8Array(proc.stdout);
} finally {rmSync(tmp,{recursive:true,force:true});}
const browserResult=compileBrowser(source);
assert.equal(browserResult.ok,true,browserResult.error);
assert.deepEqual([...native],[...browserResult.bytes],
    'native and wasm32 compiler must produce byte-identical guest Wasm');
const ordinary=compileBrowser(source,false);
assert.equal(ordinary.ok,false);
assert.match(ordinary.error,/^E_UNSUPPORTED:/);

function instantiate() {
  const mod=new WebAssembly.Module(browserResult.bytes);
  assert.deepEqual(WebAssembly.Module.imports(mod),[]);
  return new WebAssembly.Instance(mod,{}).exports;
}
const result=instantiate();
assert.equal(result['atto.preview.Worker.double.1'](21n),42n);
assert.equal(result['atto.preview.Worker.double.1'](-13n),-26n);
assert.equal(result['atto.preview.Worker.add.2'](19n,23n),42n);
assert.equal(result['atto.preview.Worker.subtract.1'](4n),3n);
assert.equal(result['atto.preview.Worker.minus.1'](-42n),42n);
assert.equal(result['atto.preview.Worker.minimum.0'](),-(1n<<63n));
assert.equal(result.panic_code.value,0);
let count=0;
for(const [name,args] of [
  ['double',[9223372036854775807n]],
  ['add',[9223372036854775807n,1n]],
  ['subtract',[-9223372036854775808n]],
  ['minus',[-9223372036854775808n]],
  ['double',[-9223372036854775808n]],
]) {
  const inst=instantiate();
  const fn=inst['atto.preview.Worker.'+name+'.'+args.length];
  assert.throws(()=>fn(...args),WebAssembly.RuntimeError);
  assert.equal(inst.panic_code.value,1,'checked overflow must set panic_code');
  count++;
}
for(const invalid of [
  'declare actor A; mtd (A self) f(int n): int { return n / 2; }',
  'declare actor A; mtd (A self) f(int n): int { return n; n += 1; }',
  'declare actor A; mtd (A self) f(): int { return 9223372036854775808; }',
  'declare actor A; mtd (A self) f(int n): int { return self.other(); }',
  'declare actor A; mtd (mut self: A) f(int n): int { return n; }',
  'declare actor A; define actor A as let int count=0; end mtd (A self) f(int n): int { return n; }',
]) {
  const rejected=compileBrowser(invalid);
  assert.equal(rejected.ok,false,invalid);
  assert.ok(rejected.error.startsWith('E_'),rejected.error);
}
console.log('PASS scalar receiver native/browser byte parity, executed methods, 5 checked overflow traps, and fail-closed checks');

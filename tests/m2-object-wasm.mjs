import assert from 'node:assert/strict';
import {readFile} from 'node:fs/promises';
// Compiler itself is real freestanding Rust wasm, no mocked parser/interpreter.
const compiler=await WebAssembly.compile(await readFile('target/wasm32-unknown-unknown/release/atto_compiler.wasm'));
assert.deepEqual(WebAssembly.Module.imports(compiler),[]);
const enc=new TextEncoder(),dec=new TextDecoder();
function compile(source) {
 const w=new WebAssembly.Instance(compiler,{}).exports;
 assert.equal(w.abi_version(),2);
 const bytes=enc.encode(source);
 assert.ok(bytes.length<=w.input_capacity());
 new Uint8Array(w.memory.buffer,w.input_ptr(),bytes.length).set(bytes);
 if(w.compile(bytes.length)!==0) {
   const data=new Uint8Array(w.memory.buffer),start=w.error_ptr();
   let end=start;while(data[end]&&end<start+512)end++;
   assert.equal(w.result_length(),0,'failed compiles cannot expose any guest bytes');
   return {diagnostic:dec.decode(data.slice(start,end))};
 }
 const guest=new Uint8Array(w.memory.buffer,w.result_ptr(),w.result_length()).slice();
 assert.equal(WebAssembly.validate(guest),true);
 return {guest};
}
async function run(source) {
 const output=compile(source);
 assert.ok(output.guest,output.diagnostic);
 const mod=await WebAssembly.compile(output.guest);
 assert.deepEqual(WebAssembly.Module.imports(mod),[],'object guests cannot import a JS allocator');
 return (await WebAssembly.instantiate(mod,{})).exports;
}
const cls=['define class Counter as',
 ' let n: int = 0;',
 ' constructor(value: int) { self.n = value; }',
 ' increment() -> int { self.n = self.n + 1; return self.n; }',
 'end'].join('\n')+'\n';
let guest=await run(cls+'pub fnc main(): int { val c = new Counter(40); return c.increment() + 1; }');
assert.equal(guest.main(),42n,'real constructor and method execution');
guest=await run(cls+'pub fnc main(): int { val a=new Counter(1); val b=new Counter(10); return a.increment()+b.increment(); }');
assert.equal(guest.main(),13n,'independent instances must not share fields');
assert.ok(guest.memory instanceof WebAssembly.Memory,'guest memory must contain the objects');
const view=new DataView(guest.memory.buffer);
assert.equal(view.getBigInt64(8,true),1n,'first class tag');
assert.equal(view.getBigInt64(16,true),2n,'first final field');
assert.equal(view.getBigInt64(24,true),1n,'second class tag');
assert.equal(view.getBigInt64(32,true),11n,'second independent field');
guest=await run('define class C as final int value: 40; fnc get() -> int {return self.value;} end\npub fnc main(): int { val c=new C(); return c.get()+2; }');
assert.equal(guest.main(),42n,'default field initializer and readonly field access');
const exhausted=await run('define class C as let n:int=1; end\npub fnc main(): int { for (let i=0; i<10000; i++) do val c=new C(); done return 0; }');
assert.throws(()=>exhausted.main(),WebAssembly.RuntimeError,'fixed heap must fail safely');
assert.equal(exhausted.panic_code.value,3,'heap exhaustion must have distinct panic code');
for (const [src,code] of [
 [cls+'pub fnc main(): int { val c=new Counter(1); val alias=c; return 0; }','E_UNSUPPORTED'],
 [cls+'pub fnc main(): int { val c=new Counter(1); return Counter.increment(c); }','E_UNSUPPORTED'],
 [cls+'pub fnc main(): int { val c=new Counter(1); Counter.constructor(c,3); return 0; }','E_UNSUPPORTED'],
 [cls+'pub fnc main(): int { val a=new Counter(1); val b=new Counter(2); if a eq b then return 1; fi return 0; }','E_UNSUPPORTED'],
 [cls+'pub fnc main(): int { const a=new Counter(1);return 0; }','E_UNSUPPORTED'],
 [cls+'pub fnc main(): int { val c=new Counter(1); return c.n; }','E_UNSUPPORTED'],
 [cls+'pub fnc main(): int { val c=new Counter(1); c.n=7; return 42; }','E_UNSUPPORTED'],
 [cls+'pub fnc main(): int { val c=new Counter(true); return 0; }','E_TYPE'],
 [cls+'pub fnc main(): int { val c=new Counter(); return 0; }','E_ARITY'],
 ['define actor A as end\npub fnc main(): int {val a=new A();return 1;}','E_UNKNOWN_TYPE'],
 ['define class C as final int x: 0; update() { self.x=2; } end\npub fnc main(): int {return 1;}','E_IMMUTABLE'],
 ['define class C as let n:int=1+2; end\npub fnc main(): int {return 1;}','E_UNSUPPORTED'],
]) {
 const output=compile(src);
 assert.equal(output.guest,undefined,src);
 assert.ok(output.diagnostic?.startsWith(code+':'),output.diagnostic);
}
console.log('PASS real compiler-Wasm -> class objects/constructors/independent field updates and safe heap exhaustion');

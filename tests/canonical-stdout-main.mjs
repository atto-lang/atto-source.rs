// Real generated Wasm, not a host-model stdout test.
import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
import {createHash} from 'node:crypto';
const [path,digest]=process.argv.slice(2);
assert.match(digest??'',/^[a-f0-9]{64}$/);
const bytes=readFileSync(path);
assert.equal(createHash('sha256').update(bytes).digest('hex'),digest);
const mod=new WebAssembly.Module(bytes);
assert.deepEqual(WebAssembly.Module.imports(mod),[]);
const api=new WebAssembly.Instance(mod,{}).exports;
assert.equal(api.abi_version(),2);
const enc=new TextEncoder(),dec=new TextDecoder('utf8',{fatal:true});
function compile(source){
  const data=enc.encode(source);
  assert.ok(data.length<=api.input_capacity());
  const ptr=api.input_ptr(),mem=new Uint8Array(api.memory.buffer);
  assert.ok(ptr>=0&&ptr+data.length<=mem.length);mem.set(data,ptr);
  const status=api.compile(data.length);
  assert.ok(status===0||status===1);
  if(status===1){
    assert.equal(api.result_length(),0);
    const m=new Uint8Array(api.memory.buffer),off=api.error_ptr();
    assert.ok(off>=0&&off<m.length);
    const end=m.indexOf(0,off);
    assert.ok(end>off&&end-off<=65536);
    return {diagnostic:dec.decode(m.subarray(off,end))};
  }
  const m=new Uint8Array(api.memory.buffer),off=api.result_ptr(),len=api.result_length();
  assert.ok(len>0&&off>=0&&off+len<=m.length&&len<=1048576);
  return {wasm:m.slice(off,off+len)};
}
let passes=0;
for(const [source,expected] of [
 ['pub fnc main(): int { stdio.out.write("A"); process.stdout.write("B"); return 42; }','AB'],
 ['pub fnc main(): int { val string s="C"; stdio.out.write(s); return 42; }','C'],
 ['pub fnc main(): int { stdio.stdout.write("D"); return 42; }','D'],
 ['pub fnc main(): int { defer stdio.out.write("A"); defer process.stdout.write("B"); stdio.out.write("C"); return 42; }','CBA']
]){
 const built=compile(source);assert.ok(built.wasm,built.diagnostic);
 const module=new WebAssembly.Module(built.wasm);assert.deepEqual(WebAssembly.Module.imports(module),[{module:'atto',name:'print_str',kind:'function'}]);
 let output='',instance;
 instance=new WebAssembly.Instance(module,{atto:{print_str(ptr,len){const mem=instance.exports.memory.buffer;assert.ok(ptr>=0&&len>=0&&ptr+len<=mem.byteLength&&output.length+len<=65536);output+=dec.decode(new Uint8Array(mem,ptr,len));}}});
 assert.equal(instance.exports.panic_code.value,0);
 assert.equal(instance.exports.main(),42n);assert.equal(instance.exports.panic_code.value,0);assert.equal(output,expected);passes++;
}
for(const [source,code] of [
 ['pub fnc main(): int { stdio.out.write(42); return 42; }','E_TYPE'],
 ['pub fnc main(): int { process.stdout.write(); return 42; }','E_ARITY'],
 ['pub fnc main(): int { val int process=1; process.stdout.write("X"); return 42; }','E_IO_SHADOW'],
 ['pub fnc main(): int { val int stdio=1; stdio.out.write("X"); return 42; }','E_IO_SHADOW']
]){
 const built=compile(source);assert.equal(built.wasm,undefined);assert.ok(built.diagnostic?.startsWith(code+':'),built.diagnostic);passes++;
}
console.log(JSON.stringify({status:'PASS',profile:'canonical-stdout-main-v1',compiler_sha256:digest,guest_executions:4,negative_denials:4,passes}));

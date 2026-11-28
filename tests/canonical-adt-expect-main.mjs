// Real generated Wasm, not a host-model ADT test.
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
const cases=[
 ['pub fnc main(): int { val Option<int> a=Some(42); val Option<int> b=None; guard a.is_some(), b.is_none() { return 1; } return a.expect("missing"); }',42n,0],
 ['pub fnc main(): int { val Result<int,string> a=Ok(42); val Result<int,string> b=Err("bad"); guard a.is_ok(), b.is_err() { return 1; } return a.expect("bad"); }',42n,0],
 ['pub fnc main(): int { val Result<int,int> a=Ok(42); val Result<int,int> b=Err(3); guard a.is_ok(), b.is_err() { return 1; } return a.expect("bad"); }',42n,0],
 ['pub fnc main(): int { val Option<int> a=None; return a.expect("missing"); }',null,9],
 ['pub fnc main(): int { val Result<int,string> a=Err("bad"); return a.expect("missing"); }',null,9],
 ['pub fnc main(): int { val Option<int> a=None; val Option<int> result=trap! a.expect("missing"); return 42; }',null,9]
];
for(const [source,value,panic] of cases){
 const built=compile(source);assert.ok(built.wasm,built.diagnostic);
 const module=new WebAssembly.Module(built.wasm);assert.deepEqual(WebAssembly.Module.imports(module),[]);
 const e=new WebAssembly.Instance(module,{}).exports;
 assert.equal(e.panic_code.value,0);
 if(panic)assert.throws(()=>e.main(),WebAssembly.RuntimeError);else assert.equal(e.main(),value);
 assert.equal(e.panic_code.value,panic);passes++;
}
for(const [source,code] of [
 ['pub fnc main(): int { val Option<int> a=None; return a.expect(); }','E_ARITY'],
 ['pub fnc main(): int { val Option<int> a=None; return a.expect(42); }','E_TYPE'],
 ['pub fnc main(): int { val Option<int> a=None; return a.is_some(1) ? 42 : 0; }','E_ARITY'],
 ['pub fnc main(): int { val Option<int> a=None; return a.is_ok() ? 42 : 0; }','E_TYPE']
]){
 const built=compile(source);assert.equal(built.wasm,undefined);assert.ok(built.diagnostic?.startsWith(code+':'),built.diagnostic);passes++;
}
console.log(JSON.stringify({status:'PASS',profile:'main-adt-expect-v1',compiler_sha256:digest,guest_executions:6,negative_denials:4,passes}));

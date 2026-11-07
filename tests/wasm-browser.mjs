import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
const compilerBytes = await readFile('target/wasm32-unknown-unknown/release/atto_compiler.wasm');
assert.equal(WebAssembly.validate(compilerBytes), true);
assert.deepEqual(WebAssembly.Module.imports(new WebAssembly.Module(compilerBytes)), []);
const { instance } = await WebAssembly.instantiate(compilerBytes, {});
const api = instance.exports;
assert.ok(api.memory);
assert.equal(api.input_capacity(),4096);
const encode = new TextEncoder(), decode = new TextDecoder();
function compile(source) {
  const data = encode.encode(source);
  assert.ok(data.length<=4096);
  new Uint8Array(api.memory.buffer,api.input_ptr(),data.length).set(data);
  const rc=api.compile(data.length);
  if(rc) {
    assert.equal(api.result_length(),0,'fail-closed compiler result');
    const raw=new Uint8Array(api.memory.buffer);
    let start=api.error_ptr(),end=start;
    while(end<raw.length && raw[end] && end-start<255)end++;
    return {ok:false,error:decode.decode(raw.subarray(start,end)),line:api.error_line_number(),column:api.error_column_number()};
  }
  const bytes=new Uint8Array(api.memory.buffer,api.result_ptr(),api.result_length()).slice();
  assert.ok(WebAssembly.validate(bytes));
  assert.deepEqual(WebAssembly.Module.imports(new WebAssembly.Module(bytes)),[]);
  return {ok:true,bytes};
}
const cases=[
  ['fnc main() -> int { const int a=6; int b=7; return a*b; }',42],
  ['fnc main() -> int { let int a=40; a=a+2; return a; }',42],
  ['fnc main() -> int { return 3 + 4 * 5; }',23],
  ['fnc main() -> int { return -6 * -7; }',42],
];
for(const [source,value] of cases){
  const result=compile(source);assert.ok(result.ok,source);
  assert.equal((await WebAssembly.instantiate(result.bytes,{})).instance.exports.main(),value);
}
for(let i=0;i<100;i++){
  const result=compile(`fnc main() -> int { return (${i} * 7) + 3; }`);
  assert.ok(result.ok);
  assert.equal((await WebAssembly.instantiate(result.bytes,{})).instance.exports.main(),i*7+3);
}
for(const [source,error] of [
  ['fnc main() -> int { int x=1; x=2; return x; }','E_IMMUTABLE'],
  ['fnc main() -> int { int x=x+1; return x; }','E_UNINITIALIZED'],
  ['fnc main() -> int { return 2 + ; }','E_EXPRESSION'],
  ['fnc main() -> int { return nope; }','E_UNBOUND'],
  ['fnc main() -> int { return 2147483648; }','E_INTEGER_RANGE'],
  ['fnc main() -> int { return 2 ^ 3; }','E_TOKEN']
]){
  const result=compile(source);assert.ok(!result.ok);assert.ok(result.error.includes(error),result.error);
}
// Mirror the existing working playground M0 negative admission contract.
for(const [source,expected] of [
  ['fnc main() -> int { int x=5; x=6; return x; }','E_IMMUTABLE'],
  ['fnc main() -> int { return foo; }','E_UNBOUND'],
  ['fnc main() -> int { return 2 + ; }','E_EXPRESSION'],
  ['fnc main() -> int { return 42 }','E_SEMICOLON'],
  ['fnc main() -> int { return 42;','E_ENTRY'],
  ['fnc main() -> int { int x=1; int x=2; return x; }','E_DUPLICATE'],
  ['fnc main() -> int { return 42; } malicious','E_TRAILING'],
  ['fnc main() -> int { return 2147483648; }','E_INTEGER_RANGE'],
  ['fnc main() -> int { return 2 ** 8; }','E_EXPRESSION'],
  ['fnc main() -> int { return 4 % 3; }','E_TOKEN'],
  ['fnc main() -> int { print(42); return 0; }','E_UNBOUND'],
  ['fnc main() -> int { return 0; }'+String.fromCharCode(0),'E_NUL'],
  ['fnc main() -> int { return 1; } fnc other() -> int { return 2; }','E_TRAILING'],
  ['fnc main() -> int { let x=1; return x; x=2; }','E_RETURN'],
  ['fnc main() -> int { }','E_RETURN'],
]){
  const result=compile(source);assert.equal(result.ok,false,source);
  assert.ok(result.error.includes(expected),source+' -> '+result.error);
}
const pos=compile(['fnc main() -> int {','  int x=1;','  x=2;','  return x;','}'].join(String.fromCharCode(10)));
assert.equal(pos.ok,false);
assert.equal(pos.line,3);
assert.equal(pos.column,3);
const trap=compile('fnc main() -> int { return 42/(3-3); }');
assert.ok(trap.ok);
assert.throws(()=>new WebAssembly.Instance(new WebAssembly.Module(trap.bytes),{}).exports.main(),WebAssembly.RuntimeError);
console.log('PASS: 111 compiler-Wasm and generated-guest executions/rejections');

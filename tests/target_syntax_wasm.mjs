// Verify the actual shipped Rust-to-Wasm compiler ABI fails closed for
// syntax-only namespace, declared-actor and out-of-line receiver headers.
import assert from 'node:assert/strict';
import {readFile} from 'node:fs/promises';

const file = process.argv[2] || 'target/wasm32-unknown-unknown/release/atto_compiler.wasm';
const compiler = new WebAssembly.Module(await readFile(file));
assert.deepEqual(WebAssembly.Module.imports(compiler), []);
const {exports: api} = new WebAssembly.Instance(compiler, {});
assert.equal(api.abi_version(), 2);
const encoder = new TextEncoder();
const decoder = new TextDecoder('utf-8', {fatal:true});

function compile(source) {
  const bytes = encoder.encode(source);
  assert.ok(bytes.length <= api.input_capacity());
  new Uint8Array(api.memory.buffer,api.input_ptr(),bytes.length).set(bytes);
  const rc = api.compile(bytes.length);
  if (rc === 0) {
    const output = new Uint8Array(api.memory.buffer,api.result_ptr(),api.result_length()).slice();
    assert.deepEqual([...output.subarray(0,8)],[0,97,115,109,1,0,0,0]);
    return {ok:true,bytes:output};
  }
  assert.equal(rc,1);
  assert.equal(api.result_length(),0,'never emit partial code on an error');
  const mem=new Uint8Array(api.memory.buffer);
  let end=api.error_ptr();
  while (mem[end]) end++;
  const text=decoder.decode(mem.subarray(api.error_ptr(),end));
  return {ok:false,code:text.split(':')[0],text,line:api.error_line_number(),col:api.error_column_number()};
}
function rejects(source,code) {
  const out=compile(source);
  assert.equal(out.ok,false,source);
  assert.equal(out.code,code,source+' -> '+out.text);
  assert.ok(out.line>=1 && out.col>=1,source);
}
rejects(`use namespace workers;
declare actor Foo1;
declare actor Foo2;
mtd (Foo1 self) foo(): void {}
mtd (Foo2 self) foo(): void {}
pub fnc main(): int { return 0; }`,'E_UNSUPPORTED');
rejects(`pub fnc helper(): int { return 1; }
declare actor Foo;
mtd (mut self: Foo) set(int n): void {}`,'E_UNSUPPORTED');
rejects('use namespace workers; pub fnc main(): int {return 1;}','E_UNSUPPORTED');
rejects('declare actor Foo; declare actor Foo;','E_DUPLICATE');
rejects('declare actor Foo; mtd (Other self) foo(): void {}','E_UNBOUND');
rejects('declare actor Foo; mtd (Foo self) foo(): void {} mtd (self: Foo) foo(): void {}','E_DUPLICATE');
rejects('declare actor Foo; mtd (Foo self) foo(int): void {}','E_SYNTAX');
rejects('declare actor Foo; use namespace workers;','E_SYNTAX');
assert.equal(compile('pub fnc main(): int {return 42;}').ok,true);
assert.equal(compile('pub fnc main(): int {stdio.println("declare actor Foo; mtd (Foo self) foo()"); return 42;}').ok,true);
console.log('PASS exact browser Wasm ABI fail-closed target syntax tests (10)');

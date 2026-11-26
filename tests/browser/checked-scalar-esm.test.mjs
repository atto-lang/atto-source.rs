import test from 'node:test';
import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';

test('actual compiler ABI emits checked scalar ESM and rejects unlowered effects',async()=>{
  const artifact=process.env.ATTO_COMPILER_WASM;
  assert.ok(artifact,'ATTO_COMPILER_WASM must point to a real compiler build');
  const {instance}=await WebAssembly.instantiate(readFileSync(artifact),{});
  const api=instance.exports;
  assert.equal(api.abi_version(),2);
  assert.equal(typeof api.compile_js,'function');
  const compile=(source,target='compile_js')=>{
    const bytes=Buffer.from(source);
    assert.ok(bytes.length<=api.input_capacity());
    new Uint8Array(api.memory.buffer,api.input_ptr(),bytes.length).set(bytes);
    const status=api[target](bytes.length);
    return {status,bytes:new Uint8Array(api.memory.buffer,api.result_ptr(),api.result_length()).slice()};
  };
  const run=async source=>{
    const js=compile(source);
    assert.equal(js.status,0,source);
    const esm=await import('data:text/javascript;base64,'+Buffer.from(js.bytes).toString('base64'));
    assert.deepEqual(esm.actorNames,[]);
    return esm.instantiate().exports;
  };
  for(const [source,expected] of [
    ['pub fnc main(): int { return 40+2; }',42n],
    ['fnc double(int x): int { return x*2; } pub fnc main(): int { return double(21); }',42n],
    ['pub fnc main(): int { if 2 < 3 then return 42; else return 0; fi }',42n],
    ['pub fnc main(): void { return; }',0n],
  ]){
    const e=await run(source);
    const wasm=compile(source,'compile');
    assert.equal(wasm.status,0);
    const guest=new WebAssembly.Instance(new WebAssembly.Module(wasm.bytes),{});
    assert.equal(e.main(),expected);
    assert.equal(guest.exports.main(),expected);
    assert.equal(e.panic_code.value,0);
  }
  const overflow=await run('pub fnc main(): int { return 9223372036854775807+1; }');
  assert.throws(()=>overflow.main(),/E_PANIC:1/);
  assert.equal(overflow.panic_code.value,1);
  for(const alias of ['suspends','susp','⧖','∂']){
    const denied=compile(alias+' fnc main(): int { return 42; }');
    assert.notEqual(denied.status,0,alias);
    assert.equal(denied.bytes.length,0,alias);
  }
  for(const source of [
    'pub fnc main(): int { return "not an int"; }',
    'pub fnc main(): int { return 1/0; }',
    'pub fnc main(): int { stdio.println(42); return 42; }'
  ]){
    const denied=compile(source);
    assert.notEqual(denied.status,0,source);
    assert.equal(denied.bytes.length,0);
  }
  assert.notEqual(api.compile_js(-1),0);
  assert.equal(api.result_length(),0);
  assert.equal((await run('pub fnc main(): int { return 42; }')).main(),42n);
});

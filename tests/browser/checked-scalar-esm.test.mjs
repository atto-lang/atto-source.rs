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
  // Compare real emitted guests, not a host-only model.
  for(const [source,expected] of [
    ['pub fnc main(): int { let s=0; loop { s=s+1; if s eq 7 then break; fi } return s; }',7n],
    ['pub fnc main(): int { let s=0; for (let i=0; i<10; i=i+1) { if i eq 4 then continue; fi s=s+i; } return s; }',41n],
    ['pub fnc main(): int { let s=0; for (let i=0; i<4; i=i+1) { for (let j=0; j<5; j=j+1) { if j eq 2 then continue; fi if j eq 4 then break; fi s=s+1; } } return s; }',12n]
  ]){
    const js=await run(source);
    const wasm=compile(source,'compile');
    assert.equal(wasm.status,0,source);
    assert.equal(js.main(),expected,source);
    assert.equal(new WebAssembly.Instance(new WebAssembly.Module(wasm.bytes),{}).exports.main(),expected,source);
  }
  let shiftCases=0;
  for(const op of ['<<<','>>>','ashr']){
    for(const a of [-(1n<<63n),-9007199254740993n,-9n,-1n,0n,1n,9n,9007199254740993n,(1n<<63n)-1n]){
      for(const n of [0n,1n,2n,31n,32n,63n]){
        const source='fnc shift(int a,int n): int { return a '+op+' n; } pub fnc main(): int { return shift(('+a+'),('+n+')); }';
        const js=await run(source);
        const wasm=compile(source,'compile');
        assert.equal(wasm.status,0,source);
        const expected=BigInt.asIntN(64,op==='<<<'?a<<n:op==='>>>'?BigInt.asUintN(64,a)>>n:a>>n);
        assert.equal(js.main(),expected,source);
        assert.equal(new WebAssembly.Instance(new WebAssembly.Module(wasm.bytes),{}).exports.main(),expected,source);
        assert.equal(js.panic_code.value,0);
        shiftCases++;
      }
    }
    for(const n of [-9223372036854775808n,-1n,64n,65n,9223372036854775807n]){
      const source='fnc shift(int a,int n): int { return a '+op+' n; } pub fnc main(): int { return shift(1,('+n+')); }';
      const js=await run(source);
      const wasm=compile(source,'compile');
      assert.equal(wasm.status,0);
      const guest=new WebAssembly.Instance(new WebAssembly.Module(wasm.bytes),{});
      assert.throws(()=>js.main(),/E_PANIC:8/);
      assert.throws(()=>guest.exports.main(),WebAssembly.RuntimeError);
      assert.equal(js.panic_code.value,8);
      assert.equal(guest.exports.panic_code.value,8);
      const bad=compile('pub fnc main(): int { return 1 '+op+' ('+n+'); }');
      assert.notEqual(bad.status,0);
      assert.equal(bad.bytes.length,0);
      shiftCases+=2;
    }
  }
  console.log('DUAL_TARGET_SHIFT_CASES='+shiftCases);
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

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
    const errorBytes=new Uint8Array(api.memory.buffer,api.error_ptr(),512);
    const end=errorBytes.indexOf(0);
    const diagnostic=status===0?'':new TextDecoder().decode(errorBytes.subarray(0,end<0?512:end));
    return {status,diagnostic,bytes:new Uint8Array(api.memory.buffer,api.result_ptr(),api.result_length()).slice()};
  };
  const run=async source=>{
    const js=compile(source);
    assert.equal(js.status,0,js.diagnostic+'\n'+source);
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
  // Capture the exact typed host calls, including binary64 signed zero and NaN.
  async function outputGuest(source,target){
    const result=compile(source,target);
    assert.equal(result.status,0,result.diagnostic+'\n'+source);
    const events=[];
    let exports;
    const atto={
      print_i64:x=>events.push(['i64',x]),
      print_bool:x=>events.push(['bool',x]),
      print_f64:x=>events.push(['f64',x]),
      print_str:(offset,length)=>events.push(['str',new TextDecoder().decode(new Uint8Array(exports.memory.buffer,offset,length))]),
      print_newline:()=>events.push(['newline'])
    };
    if(target==='compile_js'){
      const esm=await import('data:text/javascript;base64,'+Buffer.from(result.bytes).toString('base64'));
      assert.throws(()=>esm.instantiate(),WebAssembly.LinkError);
      exports=esm.instantiate(atto).exports;
    }else{
      exports=new WebAssembly.Instance(new WebAssembly.Module(result.bytes),{atto}).exports;
    }
    const value=exports.main();
    assert.equal(exports.panic_code.value,0);
    return {value,events};
  }
  for(const [file,expected] of [
    ['28-stdio-println.atto',0n],
    ['72-float-arithmetic.atto',0n],
    ['73-float-compare.atto',3n],
    ['47-eager-boolean-precedence.atto',42n]
  ]){
    const source=readFileSync(new URL('../fixtures/pass/'+file,import.meta.url),'utf8');
    const js=await outputGuest(source,'compile_js');
    const wasm=await outputGuest(source,'compile');
    assert.equal(js.value,expected,file);
    assert.deepEqual(js,wasm,file);
    assert.ok(js.events.length>0,file);
  }
  const unicode='pub fnc main(): int { stdio.println("⧖ ∂ π"); stdio.println(-0.0); stdio.println(0.0 / 0.0); return 42; }';
  const actual=await outputGuest(unicode,'compile_js');
  assert.deepEqual(actual,await outputGuest(unicode,'compile'));
  assert.deepEqual(actual,{value:42n,events:[['str','⧖ ∂ π'],['newline'],['f64',-0],['newline'],['f64',NaN],['newline']]});
  for(const file of ['74-string-values.atto','75-string-edges.atto','82-string-concat-compare.atto']){
    const source=readFileSync(new URL('../fixtures/pass/'+file,import.meta.url),'utf8');
    const js=await outputGuest(source,'compile_js');
    const wasm=await outputGuest(source,'compile');
    assert.equal(js.value,0n,file);
    assert.deepEqual(js,wasm,file);
    assert.ok(js.events.length>0,file);
  }
  // Both emitters must trap instead of growing the bounded string heap.
  const exhaustion='pub fnc main(): int { let s: string = "a"; for (let i=0; i<22; i=i+1) { s=s+s; } return 42; }';
  const jsEx=await run(exhaustion);
  const wasmEx=compile(exhaustion,'compile');
  assert.equal(wasmEx.status,0);
  const wasmGuest=new WebAssembly.Instance(new WebAssembly.Module(wasmEx.bytes),{}).exports;
  assert.equal(jsEx.memory.buffer.byteLength,1048576);
  assert.throws(()=>jsEx.main(),/E_PANIC:3/);
  assert.throws(()=>wasmGuest.main(),WebAssembly.RuntimeError);
  assert.equal(jsEx.panic_code.value,3);
  assert.equal(wasmGuest.panic_code.value,3);
  assert.equal(jsEx.memory.buffer.byteLength,wasmGuest.memory.buffer.byteLength);
  assert.throws(()=>jsEx.memory.grow(1),RangeError);
  console.log('DUAL_TARGET_OUTPUT_FIXTURES=8');
  console.log('DUAL_TARGET_STRING_HEAP_EXHAUSTION_PASS');
  // Compare both direct and mutual recursion at the exact logical edge quota.
  for(const prefix of [
    'fnc recur(int n): int { if n eq 0 then return 42; fi return recur(n-1); }',
    'fnc recur(int n): int { if n eq 0 then return 42; fi return other(n-1); } fnc other(int n): int { if n eq 0 then return 42; fi return recur(n-1); }'
  ]){
    for(const n of [0,1,1023,1024,1025]){
      const source=prefix+' pub fnc main(): int { return recur('+n+'); }';
      const js=await run(source);
      const wasm=compile(source,'compile');
      assert.equal(wasm.status,0,wasm.diagnostic);
      const guest=new WebAssembly.Instance(new WebAssembly.Module(wasm.bytes),{}).exports;
      if(n<=1024){
        assert.equal(js.main(),42n);
        assert.equal(guest.main(),42n);
        assert.equal(js.main(),42n,'quota must be released on success');
        assert.equal(guest.main(),42n);
        assert.equal(js.panic_code.value,0);
        assert.equal(guest.panic_code.value,0);
      }else{
        assert.throws(()=>js.main(),/E_PANIC:6/);
        assert.throws(()=>guest.main(),WebAssembly.RuntimeError);
        assert.equal(js.panic_code.value,6);
        assert.equal(guest.panic_code.value,6);
      }
    }
  }
  console.log('DUAL_TARGET_RECURSION_CASES=10');
  // Real checked class constructors, method calls, and independent copy!! roots.
  let classCases=0;
  for(const file of [
    '50-class-constructor-method.atto',
    '994-copy-doublebang-two-independent-class-copies.atto',
    '995-copy-doublebang-flat-class-mixed-fields.atto'
  ]){
    const source=readFileSync(new URL('../fixtures/pass/'+file,import.meta.url),'utf8');
    const js=await run(source);
    const wasm=compile(source,'compile');
    assert.equal(wasm.status,0,wasm.diagnostic);
    const guest=new WebAssembly.Instance(new WebAssembly.Module(wasm.bytes),{}).exports;
    assert.equal(js.main(),42n,file);
    assert.equal(guest.main(),42n,file);
    assert.equal(js.panic_code.value,0);
    assert.equal(guest.panic_code.value,0);
    assert.equal(js.memory.buffer.byteLength,1048576);
    assert.deepEqual(new Uint8Array(js.memory.buffer),new Uint8Array(guest.memory.buffer),file);
    const other=await run(source);
    assert.notEqual(other.memory.buffer,js.memory.buffer);
    assert.equal(other.main(),42n);
    classCases++;
  }
  console.log('DUAL_TARGET_CLASS_CASES='+classCases);
  // Ownership violations must fail in both emitters, before an artifact is published.
  let ownershipCases=0;
  for(const file of [
    '150-borrow-conflicting-loans.atto',
    '151-borrow-use-after-take.atto',
    '152-borrow-escaped-loan.atto',
    '153-copy-noncopy-class.atto',
    '155-borrow-reader-mutation.atto',
    '156-borrow-owner-move.atto',
    '157-borrow-mut-owner-read.atto',
    '991-copy-doublebang-class-unproved.atto',
    '996-copy-doublebang-exclusive-loan.atto',
    '997-copy-doublebang-nested-array-no-runtime.atto'
  ]){
    const source=readFileSync(new URL('../fixtures/fail/'+file,import.meta.url),'utf8');
    const js=compile(source,'compile_js');
    const wasm=compile(source,'compile');
    assert.notEqual(js.status,0,file);
    assert.notEqual(wasm.status,0,file);
    assert.equal(js.bytes.length,0,file);
    assert.equal(wasm.bytes.length,0,file);
    assert.equal(js.diagnostic,wasm.diagnostic,file);
    ownershipCases++;
  }
  console.log('DUAL_TARGET_OWNERSHIP_DENIAL_CASES='+ownershipCases);
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
    'pub fnc main(): int { return true; }'
  ]){
    const denied=compile(source);
    assert.notEqual(denied.status,0,source);
    assert.equal(denied.bytes.length,0);
  }
  assert.notEqual(api.compile_js(-1),0);
  assert.equal(api.result_length(),0);
  assert.equal((await run('pub fnc main(): int { return 42; }')).main(),42n);
});

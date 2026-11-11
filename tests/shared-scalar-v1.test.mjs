// Upstream Rust-source browser parity for the canonical scalar/v1 profile.
// Input source strings and Git-blob pins match oreslang-source.js/.wasm/.net/.forge.
// Does NOT certify all language features or execution on those other backends.
import test from 'node:test';
import assert from 'node:assert/strict';
import {createHash} from 'node:crypto';
import {readFile, writeFile, mkdtemp, rm} from 'node:fs/promises';
import {spawnSync} from 'node:child_process';
import {tmpdir} from 'node:os';
import {join} from 'node:path';

const MANIFEST_BLOB = 'fd9df902b2df734ac62875037ca66db3ea41bc6b';
const CASES = Object.freeze([
  {
    "id": "scalar.literal-42",
    "kind": "accept",
    "value": "42",
    "sha1": "08b48c0ccd063bc863fb35b9e59d5c2ee6bc37d4",
    "source": "pub fnc main(): int { return 42; }\n"
  },
  {
    "id": "scalar.precedence",
    "kind": "accept",
    "value": "28",
    "sha1": "2dca1f2a9510bf5648850d8b38328fe38fe9aeed",
    "source": "pub fnc main(): int { return 4 + 6 * (5 - 1); }\n"
  },
  {
    "id": "scalar.unary",
    "kind": "accept",
    "value": "-21",
    "sha1": "b018b5b8238f708f744254b5d278fe074ac60d19",
    "source": "pub fnc main(): int { return -(7 * 3); }\n"
  },
  {
    "id": "scalar.i64-min",
    "kind": "accept",
    "value": "-9223372036854775808",
    "sha1": "450ebee951733d3f0b567413e70724155d26ce13",
    "source": "pub fnc main(): int { return -9223372036854775808; }\n"
  },
  {
    "id": "scalar.i64-max",
    "kind": "accept",
    "value": "9223372036854775807",
    "sha1": "8193f4d6a5969fe0a53f0d2700ceb576033b45e3",
    "source": "pub fnc main(): int { return 9223372036854775807; }\n"
  },
  {
    "id": "scalar.i64-max-minus-one",
    "kind": "accept",
    "value": "9223372036854775806",
    "sha1": "c400f440fde2fbc10514ab7cfd2ffe3d19729ee1",
    "source": "pub fnc main(): int { return 9223372036854775807 - 1; }\n"
  },
  {
    "id": "reject.unknown",
    "kind": "reject",
    "value": null,
    "sha1": "1c3ff45f58168c3f992c3666b7e216112b5fd64c",
    "source": "pub fnc main(): int { return undeclared_value; }\n"
  },
  {
    "id": "reject.division",
    "kind": "reject",
    "value": null,
    "sha1": "74d97f3fc4cb6e0765dbf5ef378e52d90cd7187b",
    "source": "pub fnc main(): int { return 3 / 2; }\n"
  },
  {
    "id": "reject.actor",
    "kind": "reject",
    "value": null,
    "sha1": "1ef9596b87dae01b99c0196bbc90795c857c02bd",
    "source": "define actor Worker as { pub fnc on_message(int n): int { return n; } }\npub fnc main(): int { const worker = spawn Worker(); stop worker; return 43; }\n"
  },
  {
    "id": "reject.async",
    "kind": "reject",
    "value": null,
    "sha1": "5ef809067332ab47cfacbc76fb2a287de865a01c",
    "source": "pub async fnc main(): int { return 42; }\n"
  },
  {
    "id": "reject.structural",
    "kind": "reject",
    "value": null,
    "sha1": "b889c7473344a6c058b824f35bad81938d198feb",
    "source": "pub fnc main(): int { return match 2 over on 2 { 42; } end; }\n"
  }
]);
const bytes = await readFile(new URL('../target/wasm32-unknown-unknown/release/atto_compiler.wasm',import.meta.url));
assert.ok(WebAssembly.validate(bytes),'must build genuine wasm32 compiler first');
const module = new WebAssembly.Module(bytes);
assert.deepEqual(WebAssembly.Module.imports(module),[],'no host imports granted');
const api = new WebAssembly.Instance(module,{}).exports;
assert.equal(api.abi_version(),2,'compiler ABI v2');
const decoder=new TextDecoder('utf-8',{fatal:true});

function gitBlobSha1(source) {
  const b=Buffer.from(source,'utf8');
  return createHash('sha1').update('blob '+b.length+'\\0').update(b).digest('hex');
}
function compile(source) {
  const input=Buffer.from(source,'utf8');
  assert.ok(input.length>0&&input.length<=api.input_capacity(),'input budget');
  const ptr=api.input_ptr();
  assert.ok(ptr>=0&&ptr+input.length<=api.memory.buffer.byteLength,'input bounds');
  new Uint8Array(api.memory.buffer,ptr,input.length).set(input);
  const status=api.compile(input.length);
  if(status!==0) {
    assert.equal(api.result_length(),0,'rejected source must have no wasm');
    const mem=new Uint8Array(api.memory.buffer);
    const errorPtr=api.error_ptr();
    assert.ok(errorPtr>=0&&errorPtr<mem.length,'diagnostic bounds');
    let end=errorPtr;
    while(end<mem.length && end<errorPtr+512 && mem[end]!==0)end++;
    const diagnostic=decoder.decode(mem.subarray(errorPtr,end));
    return {ok:false,diagnostic,line:api.error_line_number(),column:api.error_column_number()};
  }
  const resultPtr=api.result_ptr(),len=api.result_length();
  assert.ok(len>=8 && len<=1048576 && resultPtr>=0 && resultPtr+len<=api.memory.buffer.byteLength);
  const wasm=new Uint8Array(api.memory.buffer,resultPtr,len).slice();
  assert.ok(WebAssembly.validate(wasm));
  return {ok:true,wasm};
}
function runScalarGuest(wasm) {
  const m=new WebAssembly.Module(wasm);
  assert.deepEqual(WebAssembly.Module.imports(m),[],'no implicit host capabilities');
  const exports=WebAssembly.Module.exports(m);
  assert.equal(exports.some(x=>x.name==='main'&&x.kind==='function'),true);
  const guest=new WebAssembly.Instance(m,{});
  return guest.exports.main();
}
function runNative(attoc,sourcePath) {
  const result=spawnSync(attoc,[sourcePath],{encoding:null,timeout:10000,maxBuffer:2<<20});
  assert.equal(result.error,undefined,'native compiler must start');
  assert.equal(result.signal,null,'native compiler must exit normally');
  return result;
}

test('11 canonical scalar/v1 sources are exact Git blobs',()=>{
  assert.match(MANIFEST_BLOB,/^[a-f0-9]{40}$/);
  assert.equal(CASES.length,11);
  assert.equal(CASES.filter(x=>x.kind==='accept').length,6);
  assert.equal(CASES.filter(x=>x.kind==='reject').length,5);
  for(const x of CASES){
    assert.equal(gitBlobSha1(x.source),x.sha1,'source drift: '+x.id);
  }
});

for(const row of CASES){
  test('native/wasm32 scalar/v1: '+row.id,async t=>{
    const built=compile(row.source);
    const again=compile(row.source);
    assert.equal(again.ok,built.ok);
    if(row.kind==='accept'){
      assert.equal(built.ok,true,built.diagnostic);
      assert.deepEqual(again.wasm,built.wasm,'deterministic Wasm output');
      assert.equal(runScalarGuest(built.wasm),BigInt(row.value));
    } else {
      // Legacy scalar-profile refusal. Actors with canonical ActorMail
      // declarations remain supported separately by the Rust browser compiler.
      assert.equal(built.ok,false,'negative scalar fixture unexpectedly admitted');
      assert.match(built.diagnostic,/^E_[A-Z0-9_]+:/);
      assert.ok(built.line>=1&&built.column>=1);
    }
    if(process.env.ATTOC){
      const dir=await mkdtemp(join(tmpdir(),'atto-scalar-v1-'));
      t.after(()=>rm(dir,{recursive:true,force:true}));
      const path=join(dir,row.id.replace(/[^a-z0-9-]/g,'_')+'.atto');
      await writeFile(path,row.source,{flag:'wx'});
      const native=runNative(process.env.ATTOC,path);
      if(row.kind==='accept'){
        assert.equal(native.status,0,'native frontend must accept same case');
        assert.deepEqual(new Uint8Array(native.stdout),built.wasm,'native and wasm32 compiler output must be byte-identical');
      }else{
        assert.equal(native.status,1,'native frontend must reject same case');
        assert.equal(native.stdout.length,0,'rejection cannot output Wasm');
        assert.match(native.stderr.toString('utf8'),/E_[A-Z0-9_]+:/);
      }
    }
  });
}

test('browser compiler recovers after a negative scalar fixture',()=>{
  assert.equal(compile(CASES.find(x=>x.id==='reject.unknown').source).ok,false);
  const valid=compile(CASES.find(x=>x.id==='scalar.literal-42').source);
  assert.equal(valid.ok,true);
  assert.equal(runScalarGuest(valid.wasm),42n);
});

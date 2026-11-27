import test from 'node:test';
import assert from 'node:assert/strict';
import {readFile,mkdtemp,stat,rm,writeFile} from 'node:fs/promises';
import {spawnSync} from 'node:child_process';
import {tmpdir} from 'node:os';
import {join} from 'node:path';
import {fileURLToPath} from 'node:url';
import {createHash} from 'node:crypto';

const attoc=process.env.ATTOC ||
  fileURLToPath(new URL('../target/debug/attoc',import.meta.url));
const required=process.env.ATTO_REQUIRE_PERF_GUEST==='1';
const sha256=content=>createHash('sha256').update(content).digest('hex');
const allowedImports=new Set(['print_i64','print_bool','print_str','print_newline','print_f64','actor_spawn','actor_send']);
const trap=()=>{throw new Error('unreachable host import');};
const importObject={atto:Object.fromEntries([...allowedImports].map(x=>[x,trap]))};

// Gate actual compilation behind an explicitly required guest mode.
// No synthetic transpiler, source-only regex or host model earns guest credit.
test('Rust compiler AOT emits executable Wasm for original Atto core perf policy bodies', {
  skip: !required,
  timeout:120000
},async()=>{
 const file=fileURLToPath(new URL('../core/perf_policy.atto',import.meta.url));
 const src=await readFile(file,'utf8');
 assert.match(src,/define module core as/);
 assert.match(src,/pub fnc perf_should_sample/);
 const declared=src.replace(/^([\s\S]*?)define module core as\s*/,'').replace(/\s*end\s*$/,'');
 assert.notEqual(declared,src,'module was not extracted');
 const cases=[
  ['exact-divisor','if perf_should_sample(42, 7) then return 42; fi return 0;',42n],
  ['nonmultiple','if perf_should_sample(43, 7) then return 0; fi return 42;',42n],
  ['div-by-zero','if perf_should_sample(10, 0) then return 0; fi return 42;',42n],
  ['max-i64','if perf_should_sample(9223372036854775807, 2) then return 0; fi return 42;',42n],
  ['duration','if perf_duration_ns(40, 42) == 2 then return 42; fi return 0;',42n],
  ['rollback','if perf_duration_ns(42, 40) == 0 then return 42; fi return 0;',42n],
  ['admission','if perf_can_record(true, 65535, 65536) then return 42; fi return 0;',42n],
  ['capacity-full','if perf_can_record(true, 65536, 65536) then return 0; fi return 42;',42n],
  ['drop-saturation','if perf_next_dropped(2147483647) == 2147483647 then return 42; fi return 0;',42n],
  ['probe-id','if perf_valid_probe(29) then return 42; fi return 0;',42n],
 ];
 const work=await mkdtemp(join(tmpdir(),'atto-perf-policy-'));
 try {
  for(const [name,body,want] of cases){
   const guest=declared+'\npub fnc main() -> int {\n'+body+'\n}\n';
   const input=join(work,name+'.atto'),output=join(work,name+'.wasm');
   await writeFile(input,guest,{flag:'wx',mode:0o400});
   const built=spawnSync(attoc,[input,'-o',output],{encoding:'utf8',timeout:30000,maxBuffer:1<<20});
   assert.equal(built.error,undefined, name+' compiler did not execute');
   assert.equal(built.status,0,name+' compiler diagnostics '+built.stderr);
   const bytes=await readFile(output);
   assert.ok(WebAssembly.validate(bytes),name+' invalid Wasm');
   const module=new WebAssembly.Module(bytes);
   for(const imp of WebAssembly.Module.imports(module)){
    assert.equal(imp.module,'atto',name+' unexpected module import');
    assert.ok(allowedImports.has(imp.name),name+' ambient host capability');
   }
   const instance=new WebAssembly.Instance(module,importObject);
   assert.equal(instance.exports.main(),want,name+' guest mismatch');
   assert.equal(instance.exports.panic_code.value,0,name+' unexpected panic');
   console.log('ATTO_PERF_WASM_GUEST_EXECUTED',name,sha256(bytes));
  }
  const negative=declared.replace(
    'return dropped + 1;', 'return process.monotonic_ns();'
  )+'\npub fnc main() -> int {return perf_next_dropped(4);}\n';
  assert.notEqual(negative,declared);
  const badInput=join(work,'bad.atto'),badOutput=join(work,'bad.wasm');
  await writeFile(badInput,negative,{flag:'wx',mode:0o400});
  const denied=spawnSync(attoc,[badInput,'-o',badOutput],{encoding:'utf8',timeout:30000,maxBuffer:1<<20});
  assert.notEqual(denied.status,0,'ambient clock must not become a guest import');
  await assert.rejects(stat(badOutput),{code:'ENOENT'},'rejected source emitted artifact');
  console.log('ATTO_PERF_POLICY_SHA256',sha256(src));
 }finally{await rm(work,{recursive:true,force:true});}
});

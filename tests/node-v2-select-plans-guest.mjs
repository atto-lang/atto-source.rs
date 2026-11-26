// This executes real Atto source compiled into WebAssembly by Rust attoc.
// Node only instantiates the guest; it never computes the selection outcome.
import assert from 'node:assert/strict';
import {mkdtempSync,readFileSync,rmSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {resolve,join} from 'node:path';
import {spawnSync} from 'node:child_process';

const compiler=resolve('target/debug/attoc');
const tmp=mkdtempSync(join(tmpdir(),'atto-select-plan-wasm-'));
const cases=[
  ['230-v2-static-select-fair.atto',42n,'10\n20\n'],
  ['231-v2-dynamic-select-mutate.atto',42n,'2\n'],
  ['232-v2-nb-select-snapshot.atto',42n,''],
  ['233-v2-try-select-empty.atto',42n,''],
  ['234-v2-select-single-winner.atto',42n,'0\n1\n'],
  ['235-v2-blocking-select-generation.atto',42n,''],
  ['236-v2-select-closed-loser.atto',42n,'1\n'],
  ['237-v2-select-closed-buffered.atto',42n,'0\n'],
];
try {
  for (const [name,exit,stdout] of cases) {
    const guest=join(tmp,'guest.wasm');
    const result=spawnSync(compiler,[resolve('tests/fixtures/pass',name),'-o',guest],
      {encoding:'utf8',timeout:30000,maxBuffer:128*1024});
    assert.equal(result.status,0,name+': '+result.stderr);
    const bytes=readFileSync(guest);
    assert.ok(WebAssembly.validate(bytes),name+': emitted guest is not valid Wasm');
    const mod=new WebAssembly.Module(bytes);
    const allowed=new Set(['print_i64','print_bool','print_str','print_newline','print_f64']);
    for(const imp of WebAssembly.Module.imports(mod)){
      assert.equal(imp.module,'atto',name+': unapproved import namespace');
      assert.ok(allowed.has(imp.name),name+': unapproved host capability');
    }
    let output='',instance;
    instance=new WebAssembly.Instance(mod,{atto:{
      print_i64(v){output+=String(v);},
      print_bool(v){output+=v?'true':'false';},
      print_newline(){output+='\n';},
      print_f64(v){output+=String(v);},
      print_str(ptr,len){output+=new TextDecoder('utf8',{fatal:true}).decode(
        new Uint8Array(instance.exports.memory.buffer,ptr,len));},
    }});
    assert.equal(instance.exports.main(),exit,name+': wrong guest result');
    assert.equal(instance.exports.panic_code.value,0,name+': unexpected guest panic');
    assert.equal(output,stdout,name+': wrong actual guest stdout');
    console.log('REAL_WASM_SELECT_PLAN_PASS '+name);
  }

  // Compile and execute all SelectPlan panic cases as real Wasm guests.
  // Check precise panic codes: a generic trap is insufficient evidence.
  for (const [name,expectedPanic] of [
    ['233-v2-select-empty-unwrap.atto',14],
    ['234-v2-select-all-closed.atto',9],
    ['235-v2-select-no-live-cases.atto',10],
    ['236-v2-select-pipe-owned-at-construction.atto',10],
    ['237-v2-select-pipe-claimed-after-plan.atto',10],
    ['238-v2-dynamic-select-put-pipe-owned.atto',10],
    ['239-v2-select-abandoned-future.atto',6],
    ['240-v2-select-timeout-retirement.atto',7],
    ['241-v2-select-closed-poll-retirement.atto',9],
    ['242-v2-select-admission-retirement.atto',10],
  ]) {
    const guest=join(tmp,'denied.wasm');
    const compiled=spawnSync(compiler,[resolve('tests/fixtures/panic',name),'-o',guest],
      {encoding:'utf8',timeout:30000,maxBuffer:128*1024});
    assert.equal(compiled.status,0,name+': source did not compile: '+compiled.stderr);
    const bytes=readFileSync(guest);
    assert.ok(WebAssembly.validate(bytes),name+': invalid Wasm');
    const mod=new WebAssembly.Module(bytes);
    const allowed=new Set(['print_i64','print_bool','print_str','print_newline','print_f64']);
    for (const imp of WebAssembly.Module.imports(mod)) {
      assert.equal(imp.module,'atto',name+': forbidden host import module');
      assert.ok(allowed.has(imp.name),name+': forbidden host import');
    }
    const instance=new WebAssembly.Instance(mod,{atto:{
      print_i64(){},print_bool(){},print_str(){},print_newline(){},print_f64(){},
    }});
    assert.throws(()=>instance.exports.main(),WebAssembly.RuntimeError,
      name+': expected an actual guest panic');
    assert.equal(instance.exports.panic_code.value,expectedPanic,
      name+': wrong guest panic code');
    const source=readFileSync(resolve('tests/fixtures/panic',name),'utf8');
    if(source.includes('// expect-plan-pending: 0')) {
      assert.equal(new DataView(instance.exports.memory.buffer).getBigInt64(72,true),0n,
        name+': pending plan accounting must drain before trap');
    }
    console.log('REAL_WASM_SELECT_PLAN_PANIC_'+expectedPanic+' '+name);
  }
} finally {
  rmSync(tmp,{recursive:true,force:true});
}

// Real browser-target Wasm guest tests for both guard failure-block styles.
// Run after cargo build --release --bin attoc; not parser-only evidence.
import assert from 'node:assert/strict';
import {mkdtemp, writeFile, readFile, stat, rm} from 'node:fs/promises';
import {spawnSync} from 'node:child_process';
import {tmpdir} from 'node:os';
import {join} from 'node:path';
import {createHash} from 'node:crypto';
const attoc=process.env.ATTOC||'target/release/attoc';
const temp=await mkdtemp(join(tmpdir(),'atto-guard-dual-'));
const imports={atto:{
  print_i64(){throw Error('unexpected print')},
  print_bool(){throw Error('unexpected print')},
  print_str(){throw Error('unexpected print')},
  print_newline(){throw Error('unexpected print')},
  print_f64(){throw Error('unexpected print')},
  actor_spawn(){throw Error('unexpected actor spawn')},
  actor_send(){throw Error('unexpected actor send')}
}};
const blob=data=>createHash('sha1').update(Buffer.from('blob '+Buffer.byteLength(data)+'\0')).update(data).digest('hex');
async function compile(name,source){
  const file=join(temp,name+'.atto'),output=join(temp,name+'.wasm');
  await writeFile(file,source,'utf8');
  const proc=spawnSync(attoc,[file,'-o',output],{encoding:'utf8',timeout:30_000,maxBuffer:1<<20});
  return {proc,output,sha:blob(source)};
}
try{
  const good=[
    ['brace-true',42,'pub fnc main(): int { guard true { return 7; } return 42; }'],
    ['keyword-true',42,'pub fnc main(): int { guard true else return 7; end return 42; }'],
    ['brace-false',7,'pub fnc main(): int { guard false { return 7; } return 42; }'],
    ['keyword-false',7,'pub fnc main(): int { guard false else return 7; end return 42; }'],
    ['keyword-caret',7,'pub fnc main(): int { guard false else ^ 7; end return 42; }'],
    ['keyword-nested',42,'pub fnc main(): int { if true then guard true else return 7; end else return 9; fi return 42; }'],
    ['brace-lazy',7,'fnc overflow_if_called(int v): bool { return v + 1 > 0; } pub fnc main(): int { guard false, overflow_if_called(9223372036854775807) { return 7; } return 42; }'],
    ['keyword-lazy',7,'fnc overflow_if_called(int v): bool { return v + 1 > 0; } pub fnc main(): int { guard false, overflow_if_called(9223372036854775807) else return 7; end return 42; }'],
    ['void-else-false',42,'fnc process_explicit() -> void\n    guard false else ^ void end\nend\npub fnc main(): int { process_explicit(); return 42; }'],
    ['void-brace-false',42,'fnc process_explicit() -> void\n    guard false { ^ void }\nend\npub fnc main(): int { process_explicit(); return 42; }'],
    ['void-else-true',42,'fnc process_explicit() -> void\n    guard true else ^ void end\nend\npub fnc main(): int { process_explicit(); return 42; }'],
    ['void-brace-true',42,'fnc process_explicit() -> void\n    guard true { ^ void }\nend\npub fnc main(): int { process_explicit(); return 42; }']
  ];
  for(const [name,expected,src] of good){
    const {proc,output,sha}=await compile(name,src);
    assert.equal(proc.status,0,name+': '+proc.stderr);
    const wasm=await readFile(output);
    assert.ok(WebAssembly.validate(wasm),name+': invalid guest Wasm');
    const mod=new WebAssembly.Module(wasm);
    for(const imp of WebAssembly.Module.imports(mod)){
      assert.equal(imp.module,'atto');
      assert.ok(Object.hasOwn(imports.atto,imp.name),name+': forbidden import '+imp.name);
    }
    const guest=new WebAssembly.Instance(mod,imports);
    assert.equal(guest.exports.main(),BigInt(expected),name+': guest exit');
    assert.equal(guest.exports.panic_code.value,0,name+': panic');
    console.log('PASS real guard Wasm guest',name,expected,'blob='+sha);
  }
  const bad=[
    ['keyword-fallthrough','E_GUARD','pub fnc main(): int { guard true else val x=1; end return 42; }'],
    ['brace-fallthrough','E_GUARD','pub fnc main(): int { guard true { val x=1; } return 42; }'],
    ['keyword-nonbool','E_TYPE','pub fnc main(): int { guard 7 else return 0; end return 42; }'],
    ['keyword-break','E_GUARD','pub fnc main(): int { loop { guard false else break; end return 42; } return 0; }'],
    ['brace-continue','E_GUARD','pub fnc main(): int { loop { guard false { continue; } return 42; } return 0; }'],
    ['keyword-throw','E_GUARD','pub fnc main(): int { guard false else throw Error("bad"); end return 42; }'],
    ['keyword-hidden-exit','E_GUARD','pub fnc main(): int { loop { guard false else break; return 0; end return 42; } }'],
    ['keyword-missing-end',null,'pub fnc main(): int { guard true else return 0; fi return 42; }'],
    ['void-nonvoid-else','E_RETURN','fnc process_explicit() -> int\n    guard false else ^ void end\nend\npub fnc main(): int { return process_explicit(); }'],
    ['void-nonvoid-brace','E_RETURN','fnc process_explicit() -> int\n    guard false { ^ void }\nend\npub fnc main(): int { return process_explicit(); }'],
    ['void-fallthrough-else','E_GUARD','fnc process_explicit() -> void\n    guard false else val x = 1; end\nend\npub fnc main(): int { process_explicit(); return 42; }'],
    ['void-fallthrough-brace','E_GUARD','fnc process_explicit() -> void\n    guard false { val x = 1; }\nend\npub fnc main(): int { process_explicit(); return 42; }']
  ];
  for(const [name,code,src] of bad){
    const {proc,output,sha}=await compile(name,src);
    assert.notEqual(proc.status,0,name+': invalid guard accepted');
    if(code) assert.ok(proc.stderr?.includes(code),name+': wanted '+code+', got '+proc.stderr);
    await assert.rejects(stat(output),{code:'ENOENT'},name+': generated artifact');
    console.log('PASS guard reject without Wasm',name,'blob='+sha);
  }
  assert.equal(good.length,12);assert.equal(bad.length,12);
  console.log('GUARD_DUAL_FORM_WASM_GUEST: compiled_executed=12 compile_rejected=12');
}finally{await rm(temp,{recursive:true,force:true});}

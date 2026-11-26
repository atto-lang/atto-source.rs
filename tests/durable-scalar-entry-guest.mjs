import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { existsSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
const cli=resolve(process.env.ATTOC || 'target/release/attoc');
const dir=mkdtempSync(join(tmpdir(),'atto-durable-abi-'));
const good='fnc durable_transition(int a,int b,int c)->int { return a+b+c; }\npub fnc main()->int { return 0; }';
function compile(source, mode, positive) {
  const input=join(dir,'input.atto'), output=join(dir,'out.wasm');
  writeFileSync(input,source); rmSync(output,{force:true});
  const r=spawnSync(cli,[...(mode?['--durable-scalar-v1']:[]),input,'-o',output],{cwd:dir,encoding:'utf8',timeout:30000});
  assert.equal(r.error,undefined);
  if(!positive) {
    assert.equal(r.status,1,r.stderr); assert.match(r.stderr,/E_DURABLE_SCALAR_ABI/);
    assert.equal(existsSync(output),false); return;
  }
  assert.equal(r.status,0,r.stderr);
  const bytes=readFileSync(output); assert.ok(WebAssembly.validate(bytes));
  const m=new WebAssembly.Module(bytes); assert.deepEqual(WebAssembly.Module.imports(m),[]);
  return new WebAssembly.Instance(m,{}).exports;
}
try {
  const normal=compile(good,false,true);
  assert.equal(normal.atto_durable_transition_v1,undefined);
  assert.equal(normal.main(),0n);
  const e=compile(good,true,true);
  assert.equal(e.main(),0n);
  assert.equal(typeof e.atto_durable_transition_v1,'function');
  assert.equal(e.atto_durable_transition_v1.length,3);
  assert.equal(e.atto_durable_transition_v1(1n,2n,3n),6n);
  assert.equal(e.atto_durable_transition_v1(-10n,2n,3n),-5n);
  assert.equal(e.panic_code.value,0);
  compile('pub fnc main()->int {return 0;}',true,false);
  compile(good.replace('int c','bool c').replace('a+b+c','a+b'),true,false);
  compile(good.replace('int c','int c,int d'),true,false);
  compile(good.replace('->int { return a+b+c;','->bool { return true;'),true,false);
  console.log('DURABLE_SCALAR_ABI_V1_PASS');
} finally {rmSync(dir,{recursive:true,force:true});}

// Explicit experimental-v2 CLI -> actual compiled Wasm guest in Node.
// No JS message-routing emulation. Run only after compiling native attoc:
// cargo build --locked --bin attoc && node tests/node-v2-ready-bang-guest.mjs
import assert from 'node:assert/strict';
import {mkdtempSync,readFileSync,rmSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {resolve,join} from 'node:path';
import {spawnSync} from 'node:child_process';

const compiler=resolve('target/debug/attoc');
const scratch=mkdtempSync(join(tmpdir(),'atto-v2-ready-bang-'));
const fixtures=[
  '230-v2-actor-ready-future-pipe.atto',
  '231-v2-await-spawn-ready-pipe.atto',
  '232-v2-direct-actor-pipe-bang.atto',
  '233-v2-direct-actor-rpipe-bang.atto',
  '234-v2-direct-actor-bang-duplicate-denied.atto',
];
try {
  for(const name of fixtures) {
    const fixture=resolve('tests/fixtures/pass',name);
    const guest=join(scratch,'out.wasm');
    const compiled=spawnSync(compiler,['--experimental-v2',fixture,'-o',guest],{
      encoding:'utf8',timeout:30_000,maxBuffer:128*1024
    });
    assert.equal(compiled.error,undefined, String(compiled.error));
    assert.equal(compiled.status,0,name+': experimental compiler: '+compiled.stderr);
    const wasm=readFileSync(guest);
    assert.ok(WebAssembly.validate(wasm),name+': invalid real compiled Wasm');
    const module=new WebAssembly.Module(wasm);
    const permitted=new Set(['print_i64','print_bool','print_str','print_newline','print_f64']);
    for(const imp of WebAssembly.Module.imports(module)) {
      assert.equal(imp.module,'atto',name+': unexpected imported host namespace');
      assert.ok(permitted.has(imp.name),name+': unexpected host capability '+imp.name);
    }
    let instance,output='';
    instance=new WebAssembly.Instance(module,{atto:{
      print_i64(v){output+=String(v);},
      print_bool(v){output+=v?'true':'false';},
      print_str(ptr,len){output+=new TextDecoder('utf-8',{fatal:true}).decode(
        new Uint8Array(instance.exports.memory.buffer,ptr,len));},
      print_newline(){output+='\n';},
      print_f64(v){output+=String(v);}
    }});
    assert.equal(instance.exports.main(),42n,name+': guest return');
    assert.equal(instance.exports.panic_code.value,0,name+': hidden runtime trap');
    assert.equal(output,'40\n2\n',name+': actual FIFO DATA delivery');
    console.log('REAL_V2_READY_BANG_GUEST_PASS '+name);
  }
  // Default compiler must NOT gain an invisible async fallback.
  const source=resolve('tests/fixtures/pass',fixtures[0]);
  const defaultBuild=spawnSync(compiler,['--check',source],{encoding:'utf8',timeout:30_000});
  assert.notEqual(defaultBuild.status,0,
    'default browser compiler must not silently enable experimental v2');
} finally {
  rmSync(scratch,{recursive:true,force:true});
}

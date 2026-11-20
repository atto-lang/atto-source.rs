// Real Atto .atto -> Rust compiler -> Wasm guest execution (Node 22+).
// This is NOT an algorithmic JS reference implementation: all DATA routing,
// cooperative scheduling and actor completion execute inside emitted Wasm.
// Separate Chromium tests execute the same fixtures in a browser worker.
import assert from 'node:assert/strict';
import {mkdtempSync,readFileSync,rmSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {resolve,join} from 'node:path';
import {spawnSync} from 'node:child_process';

const compiler=resolve('target/debug/attoc');
const temporary=mkdtempSync(join(tmpdir(),'atto-v2-pipe-'));
const cases=[
  ['pass/208-v2-actor-channel-actor-pipe.atto',false],
  ['pass/209-v2-actor-channel-actor-rpipe.atto',false],
  ['panic/210-v2-pipe-double-attach.atto',true],
  ['panic/211-v2-pipe-rendezvous-denial.atto',true],
];
try {
  for (const [suffix,mustPanic] of cases) {
    const source=resolve('tests/fixtures/'+suffix);
    const guestFile=join(temporary,'guest.wasm');
    const compilation=spawnSync(compiler,[source,'-o',guestFile],{
      encoding:'utf8',timeout:30000,maxBuffer:128*1024,
    });
    assert.equal(compilation.error,undefined,String(compilation.error));
    assert.equal(compilation.status,0,
      suffix+': real Atto compiler error '+compilation.stderr);
    const wasm=readFileSync(guestFile);
    assert.ok(WebAssembly.validate(wasm),suffix+': invalid Wasm artifact');
    const module=new WebAssembly.Module(wasm);
    const allowed=new Set(['print_i64','print_bool','print_str','print_newline','print_f64']);
    for (const imp of WebAssembly.Module.imports(module)) {
      assert.equal(imp.module,'atto',suffix+': untrusted host import module');
      assert.ok(allowed.has(imp.name),suffix+': unexpected host capability '+imp.name);
    }
    let output='',instance;
    const atto={
      print_i64(v){output+=String(v);},
      print_bool(v){output+=v?'true':'false';},
      print_str(ptr,len){
        output+=new TextDecoder('utf-8',{fatal:true}).decode(
          new Uint8Array(instance.exports.memory.buffer,ptr,len));
      },
      print_newline(){output+='\n';},
      print_f64(v){output+=String(v);},
    };
    instance=new WebAssembly.Instance(module,{atto});
    if (mustPanic) {
      assert.throws(()=>instance.exports.main(),WebAssembly.RuntimeError,
        suffix+': rejected graph must trap inside the actual Wasm guest');
      assert.equal(instance.exports.panic_code.value,10,
        suffix+': expected graph admission panic_code=10');
      assert.equal(output,'',suffix+': fail before any DATA delivery');
    } else {
      const result=instance.exports.main();
      assert.equal(result,42n,suffix+': await both actor completion futures');
      assert.equal(instance.exports.panic_code.value,0,
        suffix+': no hidden runtime trap');
      assert.equal(output,'40\n2\n',
        suffix+': exactly two ordered messages through capacity-1 channel');
    }
    console.log('REAL_WASM_GUEST_'+(mustPanic?'DENIAL':'PIPE')+'_PASS '+suffix);
  }
} finally {
  rmSync(temporary,{recursive:true,force:true});
}

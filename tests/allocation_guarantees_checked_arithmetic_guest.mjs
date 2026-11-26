// Actual compiler-generated Wasm: no mocked compiler or host arithmetic fallback.
// Run after cargo build --release --bin attoc:
// node tests/allocation_guarantees_checked_arithmetic_guest.mjs
import assert from 'node:assert/strict';
import { mkdtempSync, writeFileSync, readFileSync, rmSync, existsSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { spawnSync } from 'node:child_process';

const compiler = resolve(process.env.ATTOC || 'target/release/attoc');
const dir = mkdtempSync(join(tmpdir(), 'atto-memoryless-arithmetic-'));
const hi = (1n << 63n) - 1n, lo = -(1n << 63n);
const literal = n => n === lo ? '(-9223372036854775807-1)' : '(' + n + ')';
const cases = [
  ['x+1', 42n, 43n], ['x-1', 42n, 41n],
  ['x*2', 42n, 84n], ['-x', 42n, -42n],
  ['(x+2)*3', 42n, 132n],
  ['x+1', hi, null], ['x-1', lo, null],
  ['x*2', hi, null], ['-x', lo, null],
  ['x+1', lo, lo+1n], ['x-1', hi, hi-1n],
  ['x*0', lo, 0n],
];

function compile(source) {
  const input = join(dir, 'main.atto'), output = join(dir, 'guest.wasm');
  writeFileSync(input, source);
  rmSync(output, {force:true});
  const result = spawnSync(compiler, ['main.atto', '-o', output], {
    cwd:dir, encoding:'utf8', timeout:30000,
  });
  assert.ifError(result.error);
  assert.equal(result.signal, null, 'compiler terminated by signal');
  return {result, output};
}

function verifyMemoryless(bytes) {
  assert.ok(WebAssembly.validate(bytes), 'invalid compiler artifact');
  let at=8;
  const seen=new Set();
  while(at<bytes.length) {
    const id=bytes[at++];
    assert.ok([0,1,3,6,7,10].includes(id), 'forbidden section '+id);
    let size=0, done=false;
    for(let shift=0;shift<=28;shift+=7) {
      assert.ok(at<bytes.length, 'truncated section length');
      const b=bytes[at++];
      assert.ok(shift!==28||b<=15, 'section length overflow');
      size+=(b&127)*2**shift;
      if(!(b&128)){done=true;break;}
    }
    assert.ok(done && size<=bytes.length-at, 'section outside artifact');
    if(id!==0){assert.ok(!seen.has(id), 'duplicate section');seen.add(id);}
    at+=size;
  }
  for(const id of [1,3,7,10])assert.ok(seen.has(id));
  const module=new WebAssembly.Module(bytes);
  assert.deepEqual(WebAssembly.Module.imports(module), []);
  assert.ok(WebAssembly.Module.exports(module).every(e=>e.kind!=='memory'&&e.kind!=='table'));
  return module;
}

try {
  for(const [expr,arg,expected] of cases) {
    const source='@noalloc\n@nocopy\nfnc f(x:int):int { return '+expr+'; } pub fnc main():int { return f('+literal(arg)+'); }';
    const {result,output}=compile(source);
    assert.equal(result.status,0,result.stderr);
    assert.ok(existsSync(output));
    const module=verifyMemoryless(readFileSync(output));
    const instance=new WebAssembly.Instance(module,{});
    assert.equal(instance.exports.panic_code.value,0);
    if(expected===null) {
      assert.throws(()=>instance.exports.main(),WebAssembly.RuntimeError);
      assert.equal(instance.exports.panic_code.value,1,'must be integer overflow');
    } else {
      assert.equal(instance.exports.main(),expected);
      assert.equal(instance.exports.panic_code.value,0);
    }
  }
  const denied=compile('@noalloc\nfnc f(x:int):int { return x/2; } pub fnc main():int { return f(42); }');
  assert.equal(denied.result.status,1,denied.result.stderr);
  assert.ok(!existsSync(denied.output),'denial emitted an artifact');
  console.log('MEMORYLESS_CHECKED_ARITHMETIC_GUEST_PASS '+cases.length+' guests + 1 denial');
} finally { rmSync(dir,{recursive:true,force:true}); }

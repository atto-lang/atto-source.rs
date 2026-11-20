// Actual ordinary Rust-native attoc -> real Wasm scalar guest tests.
// This is deliberately NOT ByteBuffer guest success: canonical std.buffer
// must fail before emission until the authenticated class is linked.
import assert from 'node:assert/strict';
import {mkdtemp, readFile, rm, stat, writeFile} from 'node:fs/promises';
import {spawnSync} from 'node:child_process';
import {createHash} from 'node:crypto';
import {tmpdir} from 'node:os';
import {join} from 'node:path';
import {fileURLToPath} from 'node:url';

const attoc=process.env.ATTOC || 'target/release/attoc';
const tmp=await mkdtemp(join(tmpdir(),'atto-stdlib-gate-'));
const positive=[
  ['canonical-hex-effects-val-mut',
   'pub fnc main(): int throws Error { val mut n = 0x12345678; return n eq 305419896 ? 1 : 0; }',1n],
  ['grouped-effects-and-hex-underscores',
   'pub fnc main(): int throws(Error) { val n = 0x7FFF_FFFF; return n eq 2147483647 ? 1 : 0; }',1n],
  ['full-u32-but-signed-i64',
   'pub fnc main(): int { return 0xFFFF_FFFF eq 4294967295 ? 1 : 0; }',1n],
  ['mixed-effects-hex',
   'pub fnc main(): int throws Error { return 0X0F + 0x10; }',31n],
];
const negatives=[
  ['hex-no-digits','pub fnc main(): int { return 0x; }','E_INTEGER_LITERAL'],
  ['hex-bad-separator','pub fnc main(): int { return 0x1_; }','E_INTEGER_LITERAL'],
  ['hex-signed-overflow','pub fnc main(): int { return 0xFFFFFFFFFFFFFFFF; }','E_INTEGER_RANGE'],
  ['unrecognized-effect-type','pub fnc main(): int throws Wrong { return 1; }','E_TYPE'],
  ['duplicate-exception-clause','pub fnc main(): int throws Error throws(Error) { return 1; }','E_DUPLICATE'],
];
const run=(source,out)=>spawnSync(attoc,[source,'-o',out],{encoding:'utf8',timeout:30000});
try {
  for (const [label,source,expected] of positive) {
    const input=join(tmp,label+'.atto'),output=join(tmp,label+'.wasm');
    await writeFile(input,source);
    const result=run(input,output);
    assert.equal(result.status,0,label+': '+result.stderr);
    const bytes=await readFile(output);
    assert.equal(WebAssembly.validate(bytes),true,label+': not Wasm');
    const module=new WebAssembly.Module(bytes);
    assert.deepEqual(WebAssembly.Module.imports(module),[],label+': unexpected host imports');
    const guest=new WebAssembly.Instance(module,{});
    assert.equal(guest.exports.main(),expected,label+': guest result');
  }
  for (const [label,source,code] of negatives) {
    const input=join(tmp,label+'.atto'),output=join(tmp,label+'.wasm');
    await writeFile(input,source);
    const result=run(input,output);
    assert.notEqual(result.status,0,label+': invalid source accepted');
    assert.match(result.stderr,new RegExp(code),label+': wrong rejection');
    await assert.rejects(stat(output),{code:'ENOENT'},label+': failed compiler emitted artifact');
  }
  // This is byte-for-byte the shared atto-platform-conformance BB-001
  // fixture, so a future real integration can turn this negative into an
  // executable positive WITHOUT changing the input program.
  const fixture=new URL('./fixtures/byte-buffer-v1/01-u32-le-roundtrip.atto',import.meta.url);
  const exact=await readFile(fixture);
  const hash=createHash('sha1').update(Buffer.concat([
    Buffer.from('blob '+exact.byteLength+'\0'),exact])).digest('hex');
  assert.equal(hash,'7d83aa7d8a6000875fb4c73a36493053170b9932');
  const output=join(tmp,'bb001.wasm');
  const result=run(fileURLToPath(fixture),output);
  assert.notEqual(result.status,0,'canonical stdlib guest must not be replaced by a host stub');
  assert.match(result.stderr,/E_STDLIB_UNLINKED/);
  await assert.rejects(stat(output),{code:'ENOENT'});
  console.log('ATTO_BYTEBUFFER_RUST_SCALAR_GUEST_GATE_PASS scalar_guests='+positive.length+
    ' reject_cases='+negatives.length+' canonical_stdlib_guest=NOT_EXECUTED');
} finally {
  await rm(tmp,{recursive:true,force:true});
}

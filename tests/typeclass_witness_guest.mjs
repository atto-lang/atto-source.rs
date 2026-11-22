import assert from 'node:assert/strict';
import {mkdtemp,writeFile,readFile,rm,access} from 'node:fs/promises';
import {spawnSync} from 'node:child_process';
import {tmpdir} from 'node:os';
import {join} from 'node:path';
import {fileURLToPath} from 'node:url';

const attoc=process.env.ATTOC||fileURLToPath(new URL('../target/debug/attoc',import.meta.url));
const decl=[
 'define typeclass Additive',
 ' type Output;',
 ' fnc add(Self, Self): Output;',
 'end',
 'define impl Additive for int',
 ' type Output = int;',
 ' bind add = add_int;',
 'end',
 'fnc add_int(int lhs, int rhs): int { return lhs + rhs; }'
].join('\n')+'\n';
const good=decl+'pub fnc main(): int { val n: Additive.Output<int> = Additive.add<int>(20,22); return n; }\n';
const bad=[
 ['duplicate',decl+'define impl Additive for int type Output=int; bind add=add_int; end','E_WITNESS_AMBIGUOUS'],
 ['incomplete',decl.replace('bind add = add_int;',''),'E_WITNESS_INCOMPLETE'],
 ['signature',decl.replace('fnc add_int(int lhs','fnc add_int(bool lhs'),'E_WITNESS_SIGNATURE'],
 ['missing',decl+'pub fnc main(): int { return Additive.add<bool>(1,2); }','E_WITNESS_MISSING'],
 ['value',decl+'pub fnc main(): int { let w = Additive; return 0; }','E_WITNESS_VALUE'],
 ['binding',decl.replace('bind add = add_int;','bind add = absent;'),'E_WITNESS_BINDING'],
 ['member',decl+'pub fnc main(): int { return Additive.unknown<int>(1,2); }','E_WITNESS_MEMBER'],
 ['reject-old-static-method',decl+'pub fnc main(): int { return Additive::add<int>(20,22); }','E_WITNESS_STATIC_ACCESS'],
 ['reject-old-associated-type',decl+'pub fnc main(): int { val n: Additive::Output<int> = 3; return n; }','E_WITNESS_STATIC_ACCESS']
];
const dir=await mkdtemp(join(tmpdir(),'atto-typeclass-'));
try{
 const src=join(dir,'good.atto'), out=join(dir,'good.wasm'), again=join(dir,'again.wasm');
 await writeFile(src,good);
 for(const file of [out,again]){
   const r=spawnSync(attoc,[src,'-o',file],{encoding:'utf8',timeout:30000});
   assert.equal(r.status,0,r.stderr);
 }
 const bytes=await readFile(out);
 assert.ok(WebAssembly.validate(bytes));
 assert.deepEqual(bytes,await readFile(again),'compiled Wasm must be reproducible');
 const {instance}=await WebAssembly.instantiate(bytes,{atto:{}});
 assert.equal(instance.exports.main(),42n);
 assert.equal(instance.exports.panic_code.value,0);
 for(const [id,source,code] of bad){
  const file=join(dir,id+'.atto'),artifact=join(dir,id+'.wasm');
  await writeFile(file,source);
  const r=spawnSync(attoc,[file,'-o',artifact],{encoding:'utf8',timeout:30000});
  assert.notEqual(r.status,0,id+': unexpectedly compiled');
  assert.ok(r.stderr.includes(code),id+': '+r.stderr);
  await assert.rejects(access(artifact),undefined,id+': emitted invalid artifact');
 }
 console.log('PASS real Wasm witness direct call, deterministic bytes,',bad.length,'no-artifact negatives');
}finally{await rm(dir,{recursive:true,force:true});}

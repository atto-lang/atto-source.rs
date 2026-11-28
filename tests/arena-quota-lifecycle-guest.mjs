// Compiler-backed policy evidence. Concatenation does not prove import linking.
import assert from 'node:assert/strict';
import {arenaPolicyBatches} from './arena-policy-differential-v1.mjs';
import {createHash} from 'node:crypto';
import {readFileSync,writeFileSync,mkdtempSync,rmSync,existsSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {join,resolve} from 'node:path';
import {spawnSync} from 'node:child_process';
const root=new URL('../',import.meta.url);
const compiler=resolve(process.env.ATTOC||'target/release/attoc');
const files=[
 ['stdlib/policy/core/arena_limits.atto','96d14731780b041d92ff518ca9c6c125bcd3d4c7'],
 ['stdlib/policy/alloc/arena_lifecycle.atto','e4795c6d4bd561be3d913cdc617d103d0e2828c8'],
 ['tests/fixtures/arena-quota-lifecycle-guest.atto','2b1eb28f07b792f2727abe9a5691849f024e304a']
];
const bodies=files.map(([path,pin])=>{
 const bytes=readFileSync(new URL(path,root));
 assert.equal(createHash('sha1').update('blob '+bytes.length+'\0').update(bytes).digest('hex'),pin,path);
 return bytes.toString('utf8');
});
const dir=mkdtempSync(join(tmpdir(),'atto-arena-'));
function compileAndRun(source,name){
 const input=join(dir,name+'.atto'),out=join(dir,name+'.wasm');
 writeFileSync(input,source);
 assert.ok(!existsSync(out));
 const r=spawnSync(compiler,[input,'-o',out],{encoding:'utf8',timeout:60000});
 assert.equal(r.status,0,r.stderr||String(r.error));
 const bytes=readFileSync(out);
 assert.ok(WebAssembly.validate(bytes));
 const mod=new WebAssembly.Module(bytes);
 assert.deepEqual(WebAssembly.Module.imports(mod),[]);
 const instance=new WebAssembly.Instance(mod,{});
 const status=instance.exports.main();
 assert.equal(instance.exports.panic_code.value,0);
 return {status,digest:createHash('sha256').update(bytes).digest('hex')};
}
try{
 const source=bodies.join('\n');
 const first=compileAndRun(source,'policy');
 assert.equal(first.status,100n,'failed unique guest assertion: '+first.status);
 const again=compileAndRun(source,'repeat');
 assert.deepEqual(again,first,'compiler output must be deterministic');
 // Deliberately corrupt a single expectation; ensure actual guest detects it.
 const marker='arena_end_offset(0, 0, 1, 0) != 0';
 assert.equal(source.split(marker).length,2);
 const bad=compileAndRun(source.replace(marker,'arena_end_offset(0, 0, 1, 0) != 1'),'wrong-oracle');
 assert.equal(bad.status,1n);
 console.log('ARENA_POLICY_GUEST_ASSERTIONS=56 DETERMINISTIC_WASM_SHA256='+first.digest);
 console.log('WRONG_ORACLE_REJECTED_AT_ASSERTION=1');
 let tested=0;
 for(const batch of arenaPolicyBatches()){
   const result=compileAndRun(bodies.slice(0,2).join('\n')+'\n'+batch.source,'differential-'+batch.start);
   assert.equal(result.status,100n,'differential batch '+batch.start+' assertion '+result.status);
   tested+=batch.count;
 }
 assert.equal(tested,352);console.log('ARENA_POLICY_DIFFERENTIAL_WASM_PASS cases='+tested);
}finally{rmSync(dir,{recursive:true,force:true})}

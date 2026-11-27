import assert from 'node:assert/strict';
import {spawnSync} from 'node:child_process';
import {mkdtempSync,writeFileSync,readFileSync,existsSync,rmSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {join,resolve} from 'node:path';
import {createHash} from 'node:crypto';
const attoc=resolve(process.env.ATTOC??'target/debug/attoc');
assert.ok(existsSync(attoc),'native attoc required');
const dir=mkdtempSync(join(tmpdir(),'atto-contextual-new-'));
const explicit=`define class Point as
 pub final int x;
 pub final int y;
 constructor(int a, int b) {self.x=a;self.y=b;}
 pub sum():int{return self.x+self.y;}
end
pub fnc main():int{
 let p: Point = new Point(40,2);
 return p.sum();
}`;
const contextual=explicit.replace('new Point(40,2)','new {x:40,y:2}');
const run=(name,source,positive,expectedTrace=[],oracle={})=>{
 const input=join(dir,name+'.atto'), output=join(dir,name+'.wasm');
 writeFileSync(input,source);
 const r=spawnSync(attoc,[input,'-o',output],{encoding:'utf8',timeout:30000,maxBuffer:1048576});
 assert.equal(r.error,undefined,name);
 if(!positive){assert.equal(r.status,1,name+': '+r.stderr);assert.equal(existsSync(output),false,name);return null;}
 assert.equal(r.status,0,r.stderr);const bytes=readFileSync(output);
 assert.ok(WebAssembly.validate(bytes));
 const mod=new WebAssembly.Module(bytes);
 const imports=WebAssembly.Module.imports(mod);
 const trace=[];
 const expectedImports=expectedTrace.length?[['atto','print_i64','function'],['atto','print_newline','function']]:[];
 assert.deepEqual(imports.map(x=>[x.module,x.name,x.kind]).sort(),expectedImports.sort());
 const instance=new WebAssembly.Instance(mod,expectedTrace.length?{atto:{print_i64:value=>trace.push(value),print_newline:()=>{}}}:{});
 const panic=oracle.panic??0;
 if(panic) assert.throws(()=>instance.exports.main(),WebAssembly.RuntimeError,name);
 else assert.equal(instance.exports.main(),oracle.result??42n,name);
 assert.equal(instance.exports.panic_code.value,panic,name);
 assert.deepEqual(trace,expectedTrace,name);
 if(oracle.words){
   const memory=instance.exports.memory;
   assert.ok(memory instanceof WebAssembly.Memory,name);
   assert.equal(memory.buffer.byteLength,16*65536,name);
   const view=new DataView(memory.buffer);
   const actual=oracle.words.map((_,i)=>view.getBigInt64(i*8,true));
   assert.deepEqual(actual,oracle.words,name);
   if(oracle.tail){
     const tail=oracle.tail.map((_,i)=>view.getBigInt64(view.byteLength-oracle.tail.length*8+i*8,true));
     assert.deepEqual(tail,oracle.tail,name);
   }
   assert.throws(()=>memory.grow(1),RangeError,name);
 }
 console.log(JSON.stringify({name,guest_executed:true,panic_code:panic,return_value:panic?null:String(oracle.result??42n),wasm_sha256:createHash('sha256').update(bytes).digest('hex')}));
 return bytes;
};
try {
 const baseline=run('explicit',explicit,true);
 assert.deepEqual(run('contextual',contextual,true),baseline);
 assert.deepEqual(run('reordered-literals',contextual.replace('x:40,y:2','y:2,x:40'),true),baseline);
 assert.deepEqual(run('type-first',contextual.replace('p: Point','Point p'),true),baseline);
 run('reordered-expressions',contextual.replace('x:40,y:2','y:1+1,x:39+1'),true);
 const effects=contextual.replace('pub fnc main()', 'fnc mark(int n):int { stdio.println(n); return n; }\npub fnc main()').replace('x:40,y:2','y:mark(2),x:mark(40)');
 run('reordered-effects',effects,true,[2n,40n]);
 // Equal-valued reference constructions must occupy separate physical slots.
 // This is a memory-layout oracle for this fixed bump-heap profile, not a
 // language-level reference-equality or general allocator ABI guarantee.
 const two=contextual.replace('return p.sum();','let q: Point = new {x:40,y:2}; return p.sum()+q.sum();');
 run('distinct-equal-objects',two,true,[],{result:84n,words:[40n,2n,40n,2n,0n,0n]});
 // Preserve the first object, evaluate the failing second initializer once,
 // skip subsequent initializer effects and leave its destination unwritten.
 const failing=effects.replace('let p: Point = new {y:mark(2),x:mark(40)};',
   'let p: Point = new {x:11,y:31}; let q: Point = new {y:mark(9223372036854775807)+1,x:mark(40)};')
   .replace('return p.sum();','return p.sum()+q.sum();');
 assert.notEqual(failing,effects);
 assert.ok(failing.includes('let q: Point = new {y:mark(9223372036854775807)+1,x:mark(40)};'));
 run('initializer-overflow-before-construction',failing,true,[9223372036854775807n],
   {panic:1,words:[11n,31n,0n,0n]});
 // A fixed 1 MiB heap has room for precisely 65536 two-i64 objects in
 // this no-data program. The next request must trap without overwriting
 // the first object or growing memory.
 const exhausted=contextual.replace('let p: Point = new {x:40,y:2};\n return p.sum();',
   'let total=0; for(let i=0;i<65537;i++){ let p: Point = new {x:40,y:2}; total=total+p.sum(); } return total;');
 assert.notEqual(exhausted,contextual);
 const full=exhausted.replace('i<65537','i<65536');
 assert.notEqual(full,exhausted);
 run('fixed-heap-exact-capacity',full,true,[],{result:2752512n,words:[40n,2n,40n,2n],tail:[40n,2n]});
 run('fixed-heap-exhaustion',exhausted,true,[],{panic:3,words:[40n,2n,40n,2n],tail:[40n,2n]});
 for(const [name,source] of [
 ['untyped',contextual.replace('p: Point','p')],
 ['reordered-wrong-kind',contextual.replace('x:40,y:2','y:1+1,x:true')],
 ['missing',contextual.replace('x:40,y:2','x:40')],
 ['duplicate',contextual.replace('x:40,y:2','x:40,x:2')],
 ['wrong-kind',contextual.replace('x:40,y:2','x:true,y:2')],
 ['unknown-field',contextual.replace('x:40,y:2','x:40,z:2')],
 ['private',contextual.replace('pub final int x','private final int x')],
 ['non-fieldwise',contextual.replace('self.x=a','self.x=a+1')],
 ['allocator',contextual.replace('new {x:40,y:2}','new {x:40,y:2} with actor_heap')]
 ]) run(name,source,false);
 console.log('8 successful guest cases, 2 runtime panic cases, and 9 negative no-artifact checks');
} finally {rmSync(dir,{recursive:true,force:true});}

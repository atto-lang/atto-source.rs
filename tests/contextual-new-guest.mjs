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
const run=(name,source,positive,expectedTrace=[])=>{
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
 assert.deepEqual(imports.map(x=>[x.module,x.name,x.kind]),expectedTrace.length?[['atto','print_i64','function'],['atto','print_newline','function']]:[]);
 const instance=new WebAssembly.Instance(mod,expectedTrace.length?{atto:{print_i64:value=>trace.push(value),print_newline:()=>{}}}:{});
 assert.equal(instance.exports.main(),42n);
 assert.equal(instance.exports.panic_code.value,0);
 assert.deepEqual(trace,expectedTrace);
 console.log(JSON.stringify({name,guest_executed:true,return_value:'42',wasm_sha256:createHash('sha256').update(bytes).digest('hex')}));
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
 console.log('6 positive guest executions and 9 negative no-artifact checks');
} finally {rmSync(dir,{recursive:true,force:true});}

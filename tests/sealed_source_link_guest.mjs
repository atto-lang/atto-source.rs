// End-to-end linked source bytes -> native attoc-linked -> runnable guest Wasm.
// Uses an exact, locally supplied build-owned source catalogue; no remote fetch.
import assert from 'node:assert/strict';
import {mkdtempSync,writeFileSync,symlinkSync,rmSync,mkdirSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {join,resolve} from 'node:path';
import {spawnSync} from 'node:child_process';

const binary=resolve(process.env.ATTOC_LINKED || 'target/release/attoc-linked');
const dir=mkdtempSync(join(tmpdir(),'atto-source-link-'));
const units=[
  'animals/core/base=base.atto',
  'animals/pets/dog=dog.atto',
  'app/root/main=main.atto'
];
const base=`import class Dog from "animals/pets/dog";
export pub class Animal;
define sealed class Animal permits Dog as
  pub answer(): int { return 42; }
end`;
const dog=`import class Animal from "animals/core/base";
export pub class Dog;
define final class Dog extends Animal as end`;
const entry=`import class Dog from "animals/pets/dog";
pub fnc main(): int { val d = new Dog(); return d.answer(); }`;
const write=(name,content)=>writeFileSync(join(dir,name),content,'utf8');
const cmd=(files=units)=>spawnSync(binary,[
  '--root',dir,'--entry','app/root/main',
  ...files.flatMap(x=>['--unit',x])
],{encoding:null,maxBuffer:3*1024*1024});
const fail=(result,expected)=>{
  assert.notEqual(result.status,0,'malformed input must fail');
  assert.equal(result.stdout.length,0,'rejected link must never emit guest bytes');
  assert.match(result.stderr.toString(),expected);
};
try{
  write('base.atto',base);
  write('dog.atto',dog);
  write('main.atto',entry);
  const valid=cmd();
  assert.equal(valid.status,0,valid.stderr.toString());
  assert.ok(WebAssembly.validate(valid.stdout),'real linked guest must be valid Wasm');
  const module=new WebAssembly.Module(valid.stdout);
  assert.deepEqual(WebAssembly.Module.imports(module),[]);
  const guest=new WebAssembly.Instance(module,{});
  assert.equal(guest.exports.main(),42n,'real linked class guest must execute');
  assert.deepEqual(cmd(units.slice().reverse()).stdout,valid.stdout,
    'input catalogue enumeration order cannot alter the guest');
  console.log('PASS_SOURCE_LINK_GUEST_EXECUTED 42 and deterministic source reorder');

  write('dog.atto',dog.replace('extends Animal','extends Other'));
  fail(cmd(),/E_UNBOUND/);
  write('dog.atto',dog);
  write('base.atto',base.replace('permits Dog','permits Cat'));
  fail(cmd(),/E_UNBOUND/);
  write('base.atto',base);
  write('main.atto',entry.replace('new Dog()','new Unbound()'));
  fail(cmd(),/E_UNBOUND/);
  write('main.atto',entry);
  console.log('PASS_SOURCE_LINK_REJECTED no guest on broken/imported hierarchy');

  fail(cmd([...units.slice(0,2),'app/root/main=../escape.atto']),/unsafe relative source path/);
  fail(cmd([...units.slice(0,2),'app/root/main=/tmp/escape.atto']),/unsafe relative source path/);
  mkdirSync(join(dir,'sub'));
  symlinkSync(join(dir,'main.atto'),join(dir,'sub','main.atto'));
  fail(cmd([...units.slice(0,2),'app/root/main=sub/main.atto']),/symlink source components are unsupported/);
  console.log('PASS_SOURCE_LINK_ROOT_BOUNDARY traversal/symlink rejected');
}finally{
  rmSync(dir,{recursive:true,force:true});
}

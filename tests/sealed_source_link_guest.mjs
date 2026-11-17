// End-to-end linked source bytes -> native attoc-linked -> runnable guest Wasm.
// Uses an exact, locally supplied build-owned source catalogue; no remote fetch.
import assert from 'node:assert/strict';
import {createHash} from 'node:crypto';
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
  '--root',dir,'--entry','app/root/main','--allow-unpinned-local',
  ...files.flatMap(x=>['--unit',x])
],{encoding:null,maxBuffer:3*1024*1024});
const digest=(text)=>createHash('sha256').update(text,'utf8').digest('hex');
const pinned=(entries=units,pinRows=[
  'animals/core/base=1.2.0@'+digest(base),
  'animals/pets/dog=1.2.0@'+digest(dog),
  'app/root/main=2.0.0@'+digest(entry)
])=>spawnSync(binary,[
  '--root',dir,'--entry','app/root/main',
  ...entries.flatMap(x=>['--unit',x]),
  ...pinRows.flatMap(x=>['--pin',x])
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
  const linkedPinned=pinned();
  assert.equal(linkedPinned.status,0,linkedPinned.stderr.toString());
  const pinnedModule=new WebAssembly.Module(linkedPinned.stdout);
  const receipts=WebAssembly.Module.customSections(pinnedModule,'atto.source.catalogue.v1');
  assert.equal(receipts.length,1,'pinned compilation must retain one deterministic source receipt');
  assert.equal(new Uint8Array(receipts[0])[0],0,'source catalogue domain is local, not cryptographically signed');
  assert.equal(new WebAssembly.Instance(pinnedModule,{}).exports.main(),42n);
  assert.deepEqual(pinned(units.slice().reverse()).stdout,linkedPinned.stdout);
  console.log('PASS_SOURCE_LINK_PINNED_EXECUTION_SHA256 42 with deterministic manifest');

  write('dog.atto',dog+'\n// replaced after pinning');
  fail(pinned(),/E_SOURCE_DIGEST/);
  write('dog.atto',dog);
  const versionSplit=[
    'animals/core/base=1.2.0@'+digest(base),
    'animals/pets/dog=9.0.0@'+digest(dog),
    'app/root/main=2.0.0@'+digest(entry)
  ];
  fail(pinned(units,versionSplit),/E_PACKAGE_VERSION/);
  fail(pinned(units,versionSplit.slice(0,2)),/every input unit requires a pinned/);
  fail(spawnSync(binary,[
    '--root',dir,'--entry','app/root/main',
    ...units.flatMap(x=>['--unit',x])
  ],{encoding:null,maxBuffer:1024*1024}),/every input unit requires a pinned/);
  console.log('PASS_SOURCE_LINK_PIN_REJECTION mutated bytes, conflicting versions, missing pins');

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

// REAL native attoc -> checked typed IR -> guest Wasm execution. No JS matcher.
import assert from 'node:assert/strict';
import {spawnSync} from 'node:child_process';
import {createHash} from 'node:crypto';
import {existsSync,mkdtempSync,readFileSync,rmSync,writeFileSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {dirname,join,resolve} from 'node:path';
import {fileURLToPath} from 'node:url';

const here=dirname(fileURLToPath(import.meta.url));
const compiler=process.env.ATTOC ?? resolve(here,'../target/debug/attoc');
assert.ok(existsSync(compiler),'real native attoc required for guest execution');
const root=mkdtempSync(join(tmpdir(),'atto-list-match-real-guest-'));
const cases=[
  ['list-pattern-rest-60.atto',60n,[5n,10n,20n,30n,40n,50n]],
  ['list-pattern-guard-once-0.atto',0n,[5n,-1n,5n,10n,20n,30n]],
  ['list-pattern-infix-30.atto',30n,[5n,10n,20n,30n,40n,50n]],
  ['list-pattern-anchored-no-match-7.atto',7n,[3n,10n,20n,30n]],
];
const sha=buf=>createHash('sha256').update(buf).digest('hex');
const compile=(input,output)=>spawnSync(compiler,[input,'-o',output],{
  encoding:'utf8',timeout:30000,maxBuffer:1<<20,
});
try {
  for(const [file,expected,words] of cases){
    const path=join(here,'fixtures/pass',file);
    const output=join(root,file+'.wasm');
    const result=compile(path,output);
    assert.equal(result.error,undefined,file+': compiler process failed');
    assert.equal(result.status,0,file+': compile failed: '+result.stderr);
    assert.ok(existsSync(output),file+': no guest artifact');
    const bytes=readFileSync(output);
    assert.ok(WebAssembly.validate(bytes),file+': invalid generated Wasm');
    const module=new WebAssembly.Module(bytes);
    assert.deepEqual(WebAssembly.Module.imports(module),[],file+': no host list matcher is allowed');
    const instance=new WebAssembly.Instance(module,{});
    assert.equal(typeof instance.exports.main,'function',file+': no guest main');
    assert.equal(instance.exports.main(),expected,file+': wrong guest return');
    assert.equal(instance.exports.panic_code.value,0,file+': unexpected guest panic');
    assert.ok(instance.exports.memory instanceof WebAssembly.Memory,file+': no heap-backed list');
    const view=new DataView(instance.exports.memory.buffer);
    let found=0;
    for(let i=0;i+words.length*8<=view.byteLength;i+=8){
      if(words.every((value,j)=>view.getBigInt64(i+j*8,true)===value))found++;
    }
    assert.equal(found,1,file+': guest heap must contain one contiguous List header and elements');
    console.log(JSON.stringify({
      kind:'SOURCE_COMPILED_WASM_GUEST',
      source:file,source_sha256:sha(readFileSync(path)),guest_sha256:sha(bytes),
      expected_exit:String(expected),actual_exit:String(expected),imports:0,
      guest_heap_words:words.map(String),panic_code:0
    }));
  }
  const base=readFileSync(join(here,'fixtures/pass/list-pattern-rest-60.atto'),'utf8');
  const negatives=[
    ['duplicate-binding',base.replace('is int as val b','is int as val a'),'E_PATTERN_BINDING'],
    ['unbound-result',base.replace('a + b + c','a + missing + c'),'E_PATTERN_BINDING'],
    ['mismatched-element-type',base.replace('is int as val b','is bool as val b'),'E_UNSUPPORTED'],
    ['malformed-list',base.replace('10, 20, 30, 40, 50','10, 20,'),'E_PATTERN_SOURCE'],
    ['unlowered-counted',base.replace('..., is int as val a','...(3 of int), is int as val a'),'E_UNSUPPORTED'],
    ['unlowered-fragment',base.replace('..., is int as val a','pattern.of<int>("..."), is int as val a'),'E_UNSUPPORTED'],
    ['unlowered-nested-rest',base.replace('..., is int as val a','..., ..., is int as val a'),'E_UNSUPPORTED'],
  ];
  for(const [name,source,diagnostic] of negatives){
    const input=join(root,name+'.atto');
    const output=join(root,name+'.wasm');
    writeFileSync(input,source);
    writeFileSync(output,'stale artifact must be cleared');
    const result=compile(input,output);
    assert.equal(result.error,undefined,name+': compiler process failed');
    assert.notEqual(result.status,0,name+': malformed source was admitted');
    assert.ok(result.stderr.includes(diagnostic),name+': wrong diagnostic: '+result.stderr);
    assert.equal(existsSync(output),false,name+': compiler left an artifact behind');
  }
  console.log('LIST_PATTERN_COMPILED_GUEST=PASS positives=4 negatives=7');
}finally {rmSync(root,{recursive:true,force:true});}

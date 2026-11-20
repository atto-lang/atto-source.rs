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
  ['list-pattern-full-width-1.atto',1n,[3n,10n,20n,30n]],
  ['list-pattern-leftmost-3.atto',3n,[3n,1n,2n,3n]],
  ['list-pattern-max-length-32.atto',32n,[32n,1n,2n,3n,4n,5n,6n,7n,8n,9n,10n,11n,12n,13n,14n,15n,16n,17n,18n,19n,20n,21n,22n,23n,24n,25n,26n,27n,28n,29n,30n,31n,32n]],
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
  // Runtime checked overflow must trap from real generated guest Wasm.
  const overflowSource=join(here,'fixtures/pass/list-pattern-overflow-panic.atto');
  const overflowOutput=join(root,'overflow-panic.wasm');
  const overflowCompiler=compile(overflowSource,overflowOutput);
  assert.equal(overflowCompiler.error,undefined,'overflow fixture compiler error');
  assert.equal(overflowCompiler.status,0,'overflow source must compile: '+overflowCompiler.stderr);
  assert.ok(existsSync(overflowOutput));
  const overflowWasm=readFileSync(overflowOutput);
  assert.ok(WebAssembly.validate(overflowWasm),'overflow Wasm is invalid');
  const overflowModule=new WebAssembly.Module(overflowWasm);
  assert.deepEqual(WebAssembly.Module.imports(overflowModule),[],'overflow cannot use host extractor');
  const overflowGuest=new WebAssembly.Instance(overflowModule,{});
  assert.throws(()=>overflowGuest.exports.main(),WebAssembly.RuntimeError,
    'checked addition must trap rather than wrap or return');
  assert.equal(overflowGuest.exports.panic_code.value,1,'expected checked i64 overflow panic code');
  const base=readFileSync(join(here,'fixtures/pass/list-pattern-rest-60.atto'),'utf8');
  const negatives=[
    ['duplicate-binding',base.replace('is int as val b','is int as val a'),'E_PATTERN_BINDING'],
    ['unbound-result',base.replace('a + b + c','a + missing + c'),'E_PATTERN_BINDING'],
    ['mismatched-element-type',base.replace('is int as val b','is bool as val b'),'E_UNSUPPORTED'],
    ['malformed-list',base.replace('10, 20, 30, 40, 50','10, 20,'),'E_PATTERN_SOURCE'],
    ['unlowered-counted',base.replace('..., is int as val a','...(3 of int), is int as val a'),'E_UNSUPPORTED'],
    ['unlowered-fragment',base.replace('..., is int as val a','pattern.of<int>("..."), is int as val a'),'E_UNSUPPORTED'],
    ['unlowered-nested-rest',base.replace('..., is int as val a','..., ..., is int as val a'),'E_PATTERN_SOURCE'],
    ['reserved-subject',base.replace('val items: List<int>','val match: List<int>').replace('match<int> items over','match<int> match over'),'E_RESERVED'],
    ['reserved-capture',base.replace('is int as val a','is int as val return').replace('a + b + c','return + b + c'),'E_RESERVED'],
    ['unlowered-interior-rest',base.replace('is int as val b, is int as val c','is int as val b, ..., is int as val c'),'E_UNSUPPORTED'],
  ];
  for(const [name,source,diagnostic] of negatives){
    const input=join(root,name+'.atto');
    const output=join(root,name+'.wasm');
    writeFileSync(input,source);
    // An absent output is the only valid negative no-artifact proof.
    // attoc deliberately does not delete or rewrite pre-existing user files
    // when compilation fails, so never preseed this destination.
    assert.equal(existsSync(output),false,name+': test output must start absent');
    const result=compile(input,output);
    assert.equal(result.error,undefined,name+': compiler process failed');
    assert.notEqual(result.status,0,name+': malformed source was admitted');
    assert.ok(result.stderr.includes(diagnostic),name+': wrong diagnostic: '+result.stderr);
    assert.equal(existsSync(output),false,name+': failed compilation created an artifact');
  }
  // A separate regression checks the CLI's existing preservation contract:
  // compilation failures must not overwrite or delete a pre-existing file.
  // Such a stale file is NOT evidence that compilation succeeded.
  {
    const input=join(root,'preexisting-output-rejection.atto');
    const output=join(root,'preexisting-output-rejection.wasm');
    const sentinel=Buffer.from('PRESERVE_PREEXISTING_OUTPUT_ON_COMPILE_ERROR');
    writeFileSync(input,negatives[0][1]);
    writeFileSync(output,sentinel);
    const result=compile(input,output);
    assert.equal(result.error,undefined,'pre-existing output compiler process failed');
    assert.notEqual(result.status,0,'pre-existing output invalid input was admitted');
    assert.ok(result.stderr.includes(negatives[0][2]),'wrong pre-existing output diagnostic');
    assert.deepEqual(readFileSync(output),sentinel,
      'failed compilation must preserve an existing user file unchanged');
  }
  console.log('LIST_PATTERN_COMPILED_GUEST=PASS positives=7 overflow_panics=1 negatives=10');
}finally {rmSync(root,{recursive:true,force:true});}

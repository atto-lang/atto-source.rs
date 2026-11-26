// Actual attoc -> Wasm guest execution adapter. No synthetic compiler or
// imported private compiler source. Run with an independently verified attoc.
import { readFileSync, writeFileSync, mkdtempSync, mkdirSync, existsSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join, resolve, dirname } from 'node:path';
import { spawnSync } from 'node:child_process';
import { createHash } from 'node:crypto';

const args = process.argv.slice(2);
if (args.length !== 2 || args[0] !== '--compiler') {
  throw Error('usage: node tools/run-static-import-v2-rust-guests.mjs --compiler /path/to/attoc');
}
const compiler = resolve(args[1]);
const raw = readFileSync(new URL('../fixtures/import-v2/cases.json', import.meta.url));
const doc = JSON.parse(raw);
if (doc.schema !== 'atto.static-import-v2.corpus.v1' || doc.status !== 'CORPUS_ONLY_NOT_BACKEND_EXECUTION')
  throw Error('unexpected corpus schema or evidence status');
const cases = doc.cases.filter(c => c.stage === 'guest');
if (!cases.length || new Set(cases.map(c => c.id)).size !== cases.length) throw Error('invalid guest inventory');
const sha = bytes => createHash('sha256').update(bytes).digest('hex');
console.log(JSON.stringify({type:'provenance',compiler_sha256:sha(readFileSync(compiler)),corpus_sha256:sha(raw),guest_count:cases.length}));
const work = mkdtempSync(join(tmpdir(), 'atto-import-v2-'));
let passed = 0;
let failed = 0;
function checkedFile(root, name) {
  if (typeof name !== 'string' || !/^[A-Za-z0-9_-]+(?:\/[A-Za-z0-9_-]+)*\.atto$/.test(name))
    throw Error('noncanonical fixture path: ' + name);
  return join(root, name);
}
try {
  for (const [i,c] of cases.entries()) {
    const root = join(work, String(i));
    mkdirSync(root);
    const output = join(root,'guest.wasm');
    let observed;
    try {
      const fixture = c.fixture ? doc.fixtures[c.fixture] : undefined;
      if (c.fixture && !fixture?.files) throw Error('guest case has no materializable fixture');
      const entry = checkedFile(root, fixture?.entry ?? 'main.atto');
      for (const [name,source] of Object.entries(fixture?.files ?? {})) {
        const file = checkedFile(root,name);
        if(file === entry) throw Error('fixture attempts to replace entry');
        mkdirSync(dirname(file),{recursive:true});
        writeFileSync(file,source,{flag:'wx'});
      }
      mkdirSync(dirname(entry),{recursive:true});
      writeFileSync(entry,c.source,{flag:'wx'});
      const result = spawnSync(compiler,[entry,'-o',output],{
        cwd:root,encoding:'utf8',timeout:20000,maxBuffer:1024*1024,stdio:['ignore','pipe','pipe']
      });
      if(result.error || result.signal || result.status === null) throw Error('compiler process failed: '+(result.error ?? result.signal));
      const diagnostic = (result.stderr ?? '').slice(0,8192);
      const codes = [...new Set(diagnostic.match(/\bE_[A-Z_]+\b/g) ?? [])];
      observed = {exit:result.status,diagnostic_codes:codes,stderr:diagnostic,stdout:(result.stdout??'').slice(0,8192)};
      if (c.expect.outcome === 'reject') {
        if(result.status === 0 || existsSync(output) || codes.length === 0) throw Error('expected diagnosed rejection without artifact');
        if(c.expect.code && !codes.includes(c.expect.code)) throw Error('expected diagnostic '+c.expect.code);
        if(c.id==='relative-negative-escape' && !codes.includes('E_IMPORT_PATH')) throw Error('escape rejected without path diagnostic');
        observed.outcome='reject';
      } else if (c.expect.outcome === 'return') {
        if(result.status !== 0 || !existsSync(output)) throw Error('expected actual Wasm artifact');
        const bytes=readFileSync(output);
        if (!WebAssembly.validate(bytes)) throw Error('invalid Wasm');
        const module=new WebAssembly.Module(bytes);
        if(WebAssembly.Module.imports(module).length !== 0) throw Error('unexpected runtime host imports');
        // A separate Node process bounds guest execution as well as compilation.
        const guest=spawnSync(process.execPath,['-e','const b=require("node:fs").readFileSync(process.argv[1]); const v=new WebAssembly.Instance(new WebAssembly.Module(b),{}).exports.main(); console.log(String(v));',output],
          {cwd:root,encoding:'utf8',timeout:5000,maxBuffer:65536});
        if(guest.error || guest.signal || guest.status !== 0) throw Error('guest execution failed or timed out');
        const value=guest.stdout.trim();
        observed={...observed,outcome:'return',value,artifact_sha256:sha(bytes)};
        if(value!==String(c.expect.value)) throw Error('wrong guest return');
      } else throw Error('unsupported guest expectation');
      passed++;
      console.log(JSON.stringify({id:c.id,status:'PASS',...observed}));
    } catch(e) {
      failed++;
      console.log(JSON.stringify({id:c.id,status:'FAIL',reason:String(e),observed}));
    }
  }
} finally {
  rmSync(work,{recursive:true,force:true});
}
console.log(JSON.stringify({type:'summary',passed,failed,total:cases.length,scope:'actual Wasm guests and diagnosed no-artifact rejections only; not full typed/resolve parity'}));
if(failed) process.exitCode=1;

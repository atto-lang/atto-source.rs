import { readFileSync, mkdtempSync, rmSync } from 'node:fs';
import { resolve, join } from 'node:path';
import { tmpdir } from 'node:os';
import { execFileSync } from 'node:child_process';
import assert from 'node:assert/strict';
import { createStateRuntime } from './browser/state-runtime.mjs';
import {assertStateTrace} from './state-trace-oracle.mjs';
const dir = mkdtempSync(join(tmpdir(), 'atto-state-write-expression-'));
try {
  const output = join(dir, 'guest.wasm');
  execFileSync(resolve(process.env.ATTOC ?? 'target/release/attoc'),
    ['tests/fixtures/pass/state-write-expressions.atto', '-o', output]);
  const events = [], printed = [];
  const runtime = createStateRuntime({onEvent: e => events.push(e)});
  const {instance} = await WebAssembly.instantiate(readFileSync(output), {atto:{
    ...runtime.imports,
    print_i64: v => { assert.equal(runtime.snapshot().active, 0); printed.push(v); },
    print_bool: v => { assert.equal(runtime.snapshot().active, 0); printed.push(Boolean(v)); },
    print_newline: () => { assert.equal(runtime.snapshot().active, 0); },
  }});
  try { assert.equal(instance.exports.main(), 0n); runtime.complete(); }
  catch (error) { runtime.abort(); throw error; }
  assert.deepEqual(printed, [42n,43n,false,43n]);
  assert.deepEqual(events.filter(e=>e.kind==='enter'||e.kind==='leave'),
    Array.from({length:3},()=>['enter','leave'].map(kind=>({kind,containerId:1,mode:1}))).flat());
  assertStateTrace({ok:true,state:runtime.snapshot(),events}, {claims:[[1,1],[1,1],[1,1]],containers:1,terminal:'complete'});
  console.log('STATE_WRITE_EXPRESSION_OK: three balanced writes, short-circuit did not acquire, scalar results published after release');
  execFileSync(resolve(process.env.ATTOC ?? 'target/release/attoc'),
    ['tests/fixtures/pass/state-return-write-composition.atto', '-o', output]);
  const trace=[], values=[];
  const other=createStateRuntime({onEvent:e=>trace.push(e)});
  const guest=(await WebAssembly.instantiate(readFileSync(output),{atto:{
    ...other.imports,
    print_i64:v=>{assert.equal(other.snapshot().active,0);values.push(v);},
    print_newline:()=>{assert.equal(other.snapshot().active,0);}
  }})).instance;
  try{assert.equal(guest.exports.main(),0n);other.complete();}
  catch(error){other.abort();throw error;}
  assert.deepEqual(values,[42n,44n,43n]);
  assert.deepEqual(trace.filter(e=>e.kind==='enter'||e.kind==='leave'),
    [1,1,0].flatMap(mode=>['enter','leave'].map(kind=>({kind,containerId:1,mode}))));
  assertStateTrace({ok:true,state:other.snapshot(),events:trace}, {claims:[[1,1],[1,1],[1,0]],containers:1,terminal:'complete'});
  console.log('STATE_RETURN_COMPOSITION_OK: suffix arithmetic and parenthesized write results after release');
} finally { rmSync(dir,{recursive:true,force:true}); }

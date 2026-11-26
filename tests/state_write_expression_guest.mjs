import { readFileSync, mkdtempSync, rmSync } from 'node:fs';
import { resolve, join } from 'node:path';
import { tmpdir } from 'node:os';
import { execFileSync } from 'node:child_process';
import assert from 'node:assert/strict';
import { createStateRuntime } from './browser/state-runtime.mjs';
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
    print_newline: () => {},
  }});
  try { assert.equal(instance.exports.main(), 0n); runtime.complete(); }
  catch (error) { runtime.abort(); throw error; }
  assert.deepEqual(printed, [42n,43n,false,43n]);
  assert.deepEqual(events.filter(e=>e.kind==='enter'||e.kind==='leave'),
    Array.from({length:3},()=>['enter','leave'].map(kind=>({kind,containerId:1,mode:1}))).flat());
  assert.equal(runtime.snapshot().active,0);
  console.log('STATE_WRITE_EXPRESSION_OK: three balanced writes, short-circuit did not acquire, scalar results published after release');
} finally { rmSync(dir,{recursive:true,force:true}); }

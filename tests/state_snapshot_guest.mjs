// Real native-compiled Wasm proof: one ticket per tuple, not per field.
import { readFileSync, mkdtempSync, rmSync } from 'node:fs';
import { resolve, join } from 'node:path';
import { tmpdir } from 'node:os';
import { execFileSync } from 'node:child_process';
import assert from 'node:assert/strict';
import { createStateRuntime } from './browser/state-runtime.mjs';
import {assertStateTrace} from './state-trace-oracle.mjs';

const dir = mkdtempSync(join(tmpdir(), 'atto-state-snapshot-'));
try {
  const output = join(dir, 'snapshot.wasm');
  const compiler = resolve(process.env.ATTOC ?? 'target/release/attoc');
  execFileSync(compiler, ['tests/fixtures/pass/state-owned-tuple-snapshot.atto', '-o', output]);
  const events = [], printed = [];
  const runtime = createStateRuntime({ onEvent: e => events.push(e) });
  const { instance } = await WebAssembly.instantiate(readFileSync(output), {
    atto: {
      ...runtime.imports,
      print_i64: v => { assert.equal(runtime.snapshot().active, 0); printed.push(v); },
      print_newline: () => { assert.equal(runtime.snapshot().active, 0); },
    },
  });
  try {
    assert.equal(instance.exports.main(), 0n);
    runtime.complete();
  } catch (error) { runtime.abort(); throw error; }
  assert.deepEqual(printed, [42n, 1n, 50n, 2n, 0n, 0n]);
  const leases = events.filter(e => e.kind === 'enter' || e.kind === 'leave');
  const expected = [[1,1], [1,0], [1,1], [1,0], [2,0]]
    .flatMap(([containerId, mode]) => ['enter','leave'].map(kind => ({kind,containerId,mode})));
  assert.deepEqual(leases, expected);
  assertStateTrace({ok:true,state:runtime.snapshot(),events}, {claims:[[1,1],[1,0],[1,1],[1,0],[2,0]],containers:2,terminal:'complete'});
  console.log('STATE_SNAPSHOT_GUEST_OK: three one-ticket read snapshots, two balanced writes, independent owned results');
} finally { rmSync(dir, { recursive: true, force: true }); }

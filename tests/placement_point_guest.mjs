import assert from 'node:assert/strict';
import {spawnSync} from 'node:child_process';
import {mkdtempSync,readFileSync,rmSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {join} from 'node:path';
// Run through the normal native browser-compiler CLI. Conformance.mjs also
// executes the same source through the compiler built as Wasm.
const dir=mkdtempSync(join(tmpdir(),'atto-point-'));
try {
 const output=join(dir,'point.wasm');
 const r=spawnSync(process.env.ATTOC,['tests/fixtures/pass/placement-point-reuse.atto','-o',output],{encoding:'utf8'});
 assert.equal(r.status,0,r.stderr);
 const {instance}=await WebAssembly.instantiate(readFileSync(output),{});
 assert.equal(instance.exports.memory.buffer.byteLength,65536);
 for(let i=0;i<100000;i++) assert.equal(instance.exports.main(),42n);
 assert.deepEqual([...new Uint8Array(instance.exports.memory.buffer,8,16)],Array(16).fill(0));
 console.log('PLACEMENT_WASM_REPEAT=100000; FIXED_REGION_CLEARED=1');
} finally {rmSync(dir,{recursive:true,force:true});}

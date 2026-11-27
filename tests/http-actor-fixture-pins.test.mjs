// Exact source fixture admission only, never guest execution evidence.
import {test} from 'node:test';
import assert from 'node:assert/strict';
import {createHash} from 'node:crypto';
import {readFileSync} from 'node:fs';
const pins={
  'serial-1x1.atto':'7564a770934d2374ec5166a353c2c9bb3c383bdf',
  'explicit-1x1.atto':'d2e5ebaffaaa26bfe6e117aaa8b36a6eb33937e1',
  'unsupported-3x4.atto':'6987e72d70ae509522f7e050a494c0aef721a653',
};
for(const [name,sha] of Object.entries(pins))test(name+' canonical blob',()=>{
  const bytes=readFileSync(new URL('./fixtures/http-actor-v1/'+name,import.meta.url));
  const actual=createHash('sha1').update('blob '+bytes.length+'\0').update(bytes).digest('hex');
  assert.equal(actual,sha);
});

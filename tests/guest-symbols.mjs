// Execute a real generated AttoLang WebAssembly guest in two isolated
// instances: supervisor main and actor. This is not a JS approximation
// of Symbol comparisons; the actor executes the compiled guest instructions.
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';

const bytes = await readFile(process.argv[2]);
assert.equal(WebAssembly.validate(bytes), true);
const module = new WebAssembly.Module(bytes);

function entries(section) {
  const raw = WebAssembly.Module.customSections(module, section);
  assert.equal(raw.length, 1, section + ' missing');
  const data = new Uint8Array(raw[0]);
  let pos = 0;
  const leb = () => {
    let n = 0, shift = 0;
    while (true) {
      assert.ok(pos < data.length && shift <= 28, 'bad LEB');
      const b = data[pos++]; n |= (b & 127) << shift;
      if (!(b & 128)) return n;
      shift += 7;
    }
  };
  const n = leb(), found = [];
  for (let i = 0; i < n; i++) {
    if (section === 'atto.actor_mail') {
      assert.ok(pos < data.length);
      found.push(data[pos++]);
    } else {
      const len = leb();
      assert.ok(pos + len <= data.length);
      found.push(new TextDecoder('utf-8', { fatal: true }).decode(data.subarray(pos, pos + len)));
      pos += len;
    }
  }
  assert.equal(pos, data.length);
  return found;
}
assert.deepEqual(entries('atto.symbols'), ['#ready', '#stopped']);
assert.deepEqual(entries('atto.actor_mail'), [3]);

const types = [], sent = [];
const main = new WebAssembly.Instance(module, {
  atto: {
    print_i64: () => { throw new Error('supervisor never prints actor output'); },
    print_newline: () => { throw new Error('supervisor never prints actor output'); },
    actor_spawn: type => { types.push(type); return types.length - 1; },
    actor_send: (ref, value) => { sent.push({ref, value}); },
  },
});
assert.equal(main.exports.main(), 42n);
assert.deepEqual(types, [0, 0]);
assert.deepEqual(sent, [{ref:0,value:1n},{ref:1,value:1n}],
  'both actors receive the exact same statically linked #ready ID');

let output = '';
const instances = [];
for (let i = 0; i < 2; i++) {
  const actor = new WebAssembly.Instance(module, {
    atto: {
      print_i64: value => { output += String(value); },
      print_newline: () => { output += '\n'; },
      actor_spawn: () => { throw new Error('actor cannot spawn'); },
      actor_send: () => { throw new Error('actor cannot send'); },
    },
  });
  assert.notStrictEqual(main.exports.memory, actor.exports.memory);
  if (instances.length) assert.notStrictEqual(instances[0].exports.memory, actor.exports.memory);
  instances.push(actor);
  actor.exports.actor_init(types[i]);
  actor.exports.actor_receive(types[i], sent[i].value, 0n);
}
assert.equal(output, '7\n7\n');
console.log('PASS: supervisor and two isolated actor instances share one global #ready identity');

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

let actorType, sent;
const main = new WebAssembly.Instance(module, {
  atto: {
    print_i64: () => { throw new Error('supervisor never prints actor output'); },
    print_newline: () => { throw new Error('supervisor never prints actor output'); },
    actor_spawn: type => { actorType = type; return 0; },
    actor_send: (ref, value) => { assert.equal(ref, 0); sent = value; },
  },
});
assert.equal(main.exports.main(), 42n);
assert.equal(actorType, 0);
assert.equal(sent, 1n, 'sorted static #ready identity must be the first handle');

let output = '';
const actor = new WebAssembly.Instance(module, {
  atto: {
    print_i64: value => { output += String(value); },
    print_newline: () => { output += '\n'; },
    actor_spawn: () => { throw new Error('actor cannot spawn'); },
    actor_send: () => { throw new Error('actor cannot send'); },
  },
});
assert.notStrictEqual(main.exports.memory, actor.exports.memory);
actor.exports.actor_init(actorType);
actor.exports.actor_receive(actorType, sent, 0n);
assert.equal(output, '7\n');
console.log('PASS: supervisor and isolated actor execute the exact global Symbol identity');

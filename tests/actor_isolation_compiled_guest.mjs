// Execute real attoc-generated Wasm actor handlers in a deliberately small,
// trusted single-tenant Node host. This is not a production actor scheduler,
// TenantVM security sandbox, ownership refund ABI, or cross-tenant proof.
import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';

const [shared_file, bool_file] = process.argv.slice(2);
assert.ok(shared_file && bool_file, 'expected two attoc-generated .wasm paths');

function run_compiled_actor(bytes, expected_imports) {
  const module = new WebAssembly.Module(bytes);
  const imports = WebAssembly.Module.imports(module);
  const got = imports.map(x => x.module + '.' + x.name).sort();
  assert.deepEqual(got, [...expected_imports].sort(), 'unexpected guest host authority import');
  assert.ok(imports.every(x => x.module === 'atto' && x.kind === 'function'));
  const live = new Map();
  let next_id = 1;
  let output = '';
  const host = {
    print_i64: x => { output += x.toString(); },
    print_bool: x => { output += x ? 'true' : 'false'; },
    print_newline: () => { output += '\n'; },
    actor_spawn: type => {
      assert.ok(Number.isInteger(type) && type >= 0, 'invalid actor type');
      const instance = new WebAssembly.Instance(module, {atto: host});
      instance.exports.actor_init(type);
      const id = next_id++;
      live.set(id, {instance, type, sequence: 0n});
      return id;
    },
    actor_send: (id, payload) => {
      const recipient = live.get(id);
      assert.ok(recipient, 'only host-minted in-scope actor refs may be sent');
      assert.equal(typeof payload, 'bigint');
      recipient.sequence += 1n;
      recipient.instance.exports.actor_receive(recipient.type, payload, recipient.sequence);
    },
  };
  const entry = new WebAssembly.Instance(module, {atto: host});
  const result = entry.exports.main();
  assert.equal(result, 0n, 'compiled guest main return contract');
  return {live, get_output: () => output};
}

const shared = run_compiled_actor(readFileSync(shared_file),
  ['atto.actor_spawn', 'atto.actor_send', 'atto.print_i64', 'atto.print_newline']);
assert.equal(shared.live.size, 1, 'compiled main must spawn one actor');
assert.equal(shared.get_output(), '42\n',
  'compiled main ActorRef.send reaches compiled receiver Wasm handler');

const boolean = run_compiled_actor(readFileSync(bool_file),
  ['atto.actor_spawn', 'atto.print_bool', 'atto.print_newline']);
assert.equal(boolean.live.size, 1);
assert.equal(boolean.get_output(), '');
const only_actor = [...boolean.live.values()][0];
only_actor.instance.exports.actor_receive(only_actor.type, 1n, 1n);
assert.equal(boolean.get_output(), 'true\n',
  'compiled bool receive handler must evaluate and print mail.value');
console.log('PASS: compiled actor guest Wasm sends int 42 and receives bool in synthetic trusted host');
console.log('GUEST_WASM_ACTOR_HANDLER_EXECUTED=true');
console.log('PRODUCTION_ACTOR_RUNTIME_EXECUTED=false');
console.log('TENANTVM_ISOLATION_PROVED=false');

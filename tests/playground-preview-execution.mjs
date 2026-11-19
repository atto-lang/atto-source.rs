// Dedicated, strict compiled-guest admission for the *exact* public-site
// preview corpus. This does not suppress legacy-site compatibility failures
// in tests/conformance.mjs, which remains a separate required whole-suite gate.
// Native CLI vs wasm32 browser compiler bytes must match for all 43 samples.
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { spawnSync } from 'node:child_process';
import { join } from 'node:path';
import { auditPlaygroundPreviews } from './playground-preview-audit.mjs';
import { formatJavaDouble } from './java-double.mjs';
import { createStateRuntime } from './browser/state-runtime.mjs';
import { BROWSER_MAX_ACTORS, BROWSER_MAX_MAILBOX } from './actor-trace-refinement.mjs';

const here = new URL('.', import.meta.url).pathname;
const compilerPath = process.argv[2] || join(here, '../target/wasm32-unknown-unknown/release/atto_compiler.wasm');
const compilerBytes = await readFile(compilerPath);
const encoder = new TextEncoder();
const decoder = new TextDecoder('utf-8', { fatal: true });

// ---- compiler ABI v2 -------------------------------------------------------
assert.ok(WebAssembly.validate(compilerBytes), 'compiler is valid wasm');
const compilerModule = new WebAssembly.Module(compilerBytes);
assert.deepEqual(WebAssembly.Module.imports(compilerModule), [], 'compiler has no host imports');
const { exports: api } = new WebAssembly.Instance(compilerModule, {});
assert.equal(api.abi_version(), 2);

export function compile(source) {
  const bytes = encoder.encode(source);
  if (bytes.length > api.input_capacity()) return { ok: false, code: 'E_SOURCE_LIMIT' };
  new Uint8Array(api.memory.buffer, api.input_ptr(), bytes.length).set(bytes);
  const rc = api.compile(bytes.length);
  if (rc !== 0) {
    assert.equal(api.result_length(), 0, 'no artifact on error');
    const mem = new Uint8Array(api.memory.buffer);
    let end = api.error_ptr();
    while (mem[end]) end++;
    const message = decoder.decode(mem.subarray(api.error_ptr(), end));
    return { ok: false, message, code: message.split(':')[0], line: api.error_line_number(), col: api.error_column_number() };
  }
  const wasm = new Uint8Array(api.memory.buffer, api.result_ptr(), api.result_length()).slice();
  return { ok: true, wasm };
}

// ---- guest runtime (atto.browser.guest.v2) ---------------------------------
// Actors: each spawn instantiates the same module again (as the playground
// does in a dedicated Web Worker), runs actor_init, and queues mail. After
// main returns, mailboxes drain FIFO, actor by actor in spawn order: one
// valid schedule of the concurrent browser runtime.
const ALLOWED_IMPORTS = new Set(['print_i64', 'print_bool', 'print_str', 'print_newline', 'print_f64', 'actor_spawn', 'actor_send',
  'state_register', 'state_enter', 'state_leave']);
const ALLOWED_EXPORTS = new Set(['main', 'panic_code', 'memory', 'actor_init', 'actor_receive']);
export const MAX_ACTORS = BROWSER_MAX_ACTORS;
export const MAX_MAILBOX = BROWSER_MAX_MAILBOX;

export function actorNames(module) {
  const sections = WebAssembly.Module.customSections(module, 'atto.actors');
  if (!sections.length) return [];
  assert.equal(sections.length, 1, 'one atto.actors section');
  const bytes = new Uint8Array(sections[0]);
  let pos = 0;
  const leb = () => { let r = 0, s = 0, b; do { b = bytes[pos++]; r |= (b & 127) << s; s += 7; } while (b & 128); return r; };
  const names = [];
  for (let n = leb(), i = 0; i < n; i++) { const len = leb(); names.push(decoder.decode(bytes.subarray(pos, pos + len))); pos += len; }
  assert.equal(pos, bytes.length, 'atto.actors has no trailing bytes');
  return names;
}

export function runGuest(wasm, { onActorTrace = null } = {}) {
  const emit = event => { if (onActorTrace) onActorTrace(event); };
  assert.ok(WebAssembly.validate(wasm), 'guest is valid wasm');
  const module = new WebAssembly.Module(wasm);
  for (const imp of WebAssembly.Module.imports(module)) {
    assert.equal(imp.module, 'atto');
    assert.ok(ALLOWED_IMPORTS.has(imp.name), `unexpected import ${imp.name}`);
  }
  const exportNames = WebAssembly.Module.exports(module).map(e => e.name).sort();
  assert.ok(['main', 'panic_code'].every(n => exportNames.includes(n)), 'guest exports main and panic_code');
  assert.ok(exportNames.every(n => ALLOWED_EXPORTS.has(n)), 'no extra exports');
  const names = actorNames(module);
  assert.equal(exportNames.includes('actor_receive'), names.length > 0, 'actor exports iff actor table');
  let out = '';
  const actors = [];
  const runtimes = new WeakMap();
  const instantiate = inActor => {
    let instance;
    const state = createStateRuntime();
    const atto = {
      print_i64: v => { out += String(v); },
      print_bool: v => { out += v ? 'true' : 'false'; },
      print_str: (ptr, len) => { out += decoder.decode(new Uint8Array(instance.exports.memory.buffer, ptr, len)); },
      print_newline: () => { out += '\n'; },
      print_f64: v => { out += formatJavaDouble(v); },
      actor_spawn: type => {
        if (inActor) throw new Error('spawn inside an actor');
        if (!(type >= 0 && type < names.length)) throw new Error('unknown actor type');
        if (actors.length >= MAX_ACTORS) throw new Error('too many actors');
        const a = { type, instance: instantiate(true), queue: [], queueIds: [],
          id: actors.length, generation: 1, nextMessageId: 1 };
        a.instance.exports.actor_init(type);
        actors.push(a);
        emit({ kind: 'actor_spawn', actor_id: a.id, generation: a.generation });
        return actors.length - 1;
      },
      actor_send: (ref, value) => {
        if (inActor) throw new Error('send inside an actor');
        const a = actors[ref];
        if (!a) throw new Error('bad actor ref');
        if (a.queue.length >= MAX_MAILBOX) throw new Error('mailbox full');
        const messageId = a.nextMessageId++;
        a.queue.push(value);
        a.queueIds.push(messageId);
        emit({ kind: 'send_accept', actor_id: a.id, generation: a.generation,
          message_id: messageId });
      },
      ...state.imports,
    };
    instance = new WebAssembly.Instance(module, { atto });
    runtimes.set(instance, state);
    return instance;
  };
  const main = instantiate(false);
  let value;
  try {
    value = main.exports.main();
    assert.equal(typeof value, 'bigint');
    runtimes.get(main).complete();
  } catch (error) {
    runtimes.get(main).abort();
    return { ok: false, panic: main.exports.panic_code.value, error: String(error), stdout: out };
  }
  for (const a of actors) {
    for (let seq = 0; seq < a.queue.length; seq++) {
      const messageId = a.queueIds[seq];
      emit({ kind: 'receive_start', actor_id: a.id, generation: a.generation,
        message_id: messageId });
      try { a.instance.exports.actor_receive(a.type, a.queue[seq], BigInt(seq)); }
      catch (error) {
        runtimes.get(a.instance).abort();
        return { ok: false, panic: a.instance.exports.panic_code.value, actor: names[a.type], error: String(error), stdout: out };
      }
      emit({ kind: 'receive_complete', actor_id: a.id, generation: a.generation,
        message_id: messageId });
    }
    runtimes.get(a.instance).complete();
    emit({ kind: 'actor_close', actor_id: a.id, generation: a.generation });
  }
  return { ok: true, value, stdout: out, actors: actors.length };
}


const attoc = process.env.ATTOC;
assert.ok(attoc, 'ATTOC must name the exact-tree native compiler for byte parity');
function nativeBytes(path) {
  const r = spawnSync(attoc, [path], { maxBuffer: 1 << 22 });
  return r.status === 0 ? new Uint8Array(r.stdout) : null;
}
const result = await auditPlaygroundPreviews({ compile, runGuest, nativeBytes });
assert.equal(result.total, 43);
assert.equal(result.guestExecuted.length, 23);
assert.equal(result.executionCoverage.remaining, 20);
assert.equal(result.nativeCompared, 43);
assert.equal(result.unsupported.length, 20);
assert.deepEqual(result.unverified, []);
assert.ok(result.guestExecuted.includes('lambdas'));
assert.ok(result.unsupported.some(x => x.name === 'lambdas-advanced'));
console.log('PASS exact-site preview corpus: 23 compiled guest oracles, 20 fail-closed previews; 43 native/Wasm compiler comparisons');

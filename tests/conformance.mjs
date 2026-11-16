// Conformance harness for the browser AttoLang compiler.
//
// Every fixture is compiled by the real wasm32 compiler artifact (the same
// bytes the playground ships), and every produced guest module is executed
// in a WebAssembly VM. Optional differential layers:
//   ATTOC=<path>      native build of the same compiler; outputs must be byte-identical
//   ATTO_JAVA=<path>  oreslang-source.java launcher; exit status and stdout must agree
//
// usage: node tests/conformance.mjs [compiler.wasm]
import assert from 'node:assert/strict';
import { auditPlaygroundPreviews } from './playground-preview-audit.mjs';
import { readFile, readdir } from 'node:fs/promises';
import { execFileSync, spawnSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { join } from 'node:path';
import { formatJavaDouble } from './java-double.mjs';
import {createStateRuntime} from './browser/state-runtime.mjs';
import { verify_actor_trace, BROWSER_MAX_ACTORS, BROWSER_MAX_MAILBOX } from './actor-trace-refinement.mjs';

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

function header(source, key) {
  return [...source.matchAll(new RegExp(`^// ${key}: ?(.*)$`, 'gm'))].map(m => m[1]);
}
async function fixtures(kind) {
  const dir = join(here, 'fixtures', kind);
  const names = (await readdir(dir)).filter(n => n.endsWith('.atto')).sort();
  return Promise.all(names.map(async name => ({ name, path: join(dir, name), source: await readFile(join(dir, name), 'utf8') })));
}

const java = process.env.ATTO_JAVA;
const attoc = process.env.ATTOC;
// The Java runtime sometimes aborts at exit while an actor is still draining
// ("context is currently executing on another thread"): stdout is then a
// truncated prefix and the status is 1. That is a runtime shutdown race, not
// a rejection, so only acceptance and the stdout prefix are compared.
const JAVA_SHUTDOWN_RACE = 'The context is currently executing on another thread';
function runJava(path) {
  const r = spawnSync(java, [path], { encoding: 'utf8', timeout: 60_000 });
  const all = r.stderr + r.stdout;
  const race = all.includes(JAVA_SHUTDOWN_RACE);
  const rejected = all.split('\n').some(l => /oreslang: /.test(l) && !l.includes(JAVA_SHUTDOWN_RACE));
  return { status: r.status, stdout: r.stdout, rejected, race };
}
function nativeBytes(path) {
  const r = spawnSync(attoc, [path], { maxBuffer: 1 << 22 });
  return r.status === 0 ? new Uint8Array(r.stdout) : null;
}

let passed = 0, javaChecked = 0, nativeChecked = 0;
let tracedActorMessages = 0, tracedActorInstances = 0;
const failures = [];
function check(label, fn) {
  try { fn(); passed++; } catch (e) { failures.push(`${label}: ${e.message}`); }
}
const digests = [];


// ---- reviewed site source: compiler byte parity and real guest outcomes ----
const siteDir = join(here, 'playground_site_examples');
const siteManifest = JSON.parse(await readFile(join(siteDir, 'manifest.json'), 'utf8'));
assert.equal(siteManifest.schema, 'atto.compiler.playground-exact-source.v1');
assert.equal(siteManifest.site_repository, 'atto-lang/atto-lang.github.io');
assert.match(siteManifest.site_commit, /^[0-9a-f]{40}$/);
assert.deepEqual(siteManifest.cases.map(row => row.name).sort(),
  ['actors','classes','conditions','enums','error','fib','hello','overflow','primes']);
assert.deepEqual((await readdir(siteDir)).filter(name => name.endsWith('.atto')).sort(),
  siteManifest.cases.map(row => row.path).sort());
let siteExecuted = 0;
for (const row of siteManifest.cases) {
  const path = join(siteDir, row.path);
  const bytes = await readFile(path);
  const source = decoder.decode(bytes);
  check('playground-site/' + row.name, () => {
    assert.match(row.site_blob, /^[a-f0-9]{40}$/, 'expected exact upstream Git blob');
    const blobSha = createHash('sha1')
      .update('blob ' + bytes.length + '\0').update(bytes).digest('hex');
    assert.equal(blobSha, row.site_blob, 'unreviewed source drift');
    const compiled = compile(source);
    if (row.kind === 'reject') {
      assert.equal(compiled.ok, false, 'rejected source must fail to compile');
      assert.equal(compiled.code, row.error_code);
      assert.equal('wasm' in compiled, false, 'rejected source must emit no guest');
      if (attoc) assert.equal(nativeBytes(path), null, 'native compiler must reject');
      return;
    }
    assert.equal(compiled.ok, true, compiled.message);
    assert.ok(WebAssembly.validate(compiled.wasm), 'real compiled guest module');
    assert.deepEqual(compile(source).wasm, compiled.wasm, 'deterministic guest Wasm');
    if (attoc) assert.deepEqual(nativeBytes(path), compiled.wasm,
      'native and wasm32 compiler outputs must match byte for byte');
    const actorTrace = [];
    const guest = runGuest(compiled.wasm, {
      onActorTrace: event => actorTrace.push(event)
    });
    if (row.kind === 'panic') {
      assert.equal(guest.ok, false, 'checked arithmetic must panic, not wrap');
      assert.equal(guest.panic, row.panic_code, 'guest panic category');
      assert.ok(guest.stdout.endsWith(row.stdout_suffix), 'stdout before panic preserved');
    } else {
      assert.equal(row.kind, 'execute');
      assert.equal(guest.ok, true, guest.error);
      assert.equal(guest.value, BigInt(row.exit), 'main result');
      assert.equal(guest.stdout, row.stdout, 'exact sample stdout');
      if (row.actor_count !== undefined) {
        const verified = verify_actor_trace(actorTrace);
        assert.equal(verified.actor_count, row.actor_count, 'real actor instances');
        assert.equal(verified.completed, row.actor_messages, 'real mailbox deliveries');
      }
    }
    siteExecuted++;
  });
}
console.log('SITE_PLAYGROUND_GUESTS_EXECUTED=' + siteExecuted +
  ' upstream_site_sha=' + siteManifest.site_commit);


for (const f of await fixtures('pass')) {
  check(`pass/${f.name}`, () => {
    const r = compile(f.source);
    assert.ok(r.ok, `expected success, got ${r.message} at ${r.line}:${r.col}`);
    const again = compile(f.source);
    assert.deepEqual(again.wasm, r.wasm, 'deterministic output');
    digests.push(`${createHash('sha256').update(r.wasm).digest('hex')}  ${f.name}`);
    const actorTrace = [];
    const g = runGuest(r.wasm, { onActorTrace: event => actorTrace.push(event) });
    assert.ok(g.ok, `guest trapped: ${g.error}`);
    const verified = verify_actor_trace(actorTrace);
    assert.equal(verified.actor_count, g.actors, 'compiled guest actors match host trace');
    tracedActorInstances += verified.actor_count;
    tracedActorMessages += verified.completed;
    assert.equal(g.value, BigInt(header(f.source, 'expect-exit')[0]), 'main() result');
    const want = header(f.source, 'expect-stdout');
    assert.equal(g.stdout, want.length ? want.join('\n') + '\n' : '', 'stdout');
    if (attoc) {
      assert.deepEqual(nativeBytes(f.path), r.wasm, 'native and wasm32 compiler hosts emit identical bytes');
      nativeChecked++;
    }
    if (java) {
      const j = runJava(f.path);
      const claim = header(f.source, 'java')[0];
      if (claim === 'llvm-design') {
        // spelling from the oreslang-source.llvm v1 design contracts; tracked
        // so this fixture is revisited when the Java parser admits it
        assert.ok(j.rejected, 'Java now accepts this LLVM-design spelling: update the fixture claim');
      } else {
        assert.ok(!j.rejected, `Java reference rejected it: ${j.stdout}`);
        if (j.race && header(f.source, 'stdout-order')[0] === 'unordered') {
          // truncated and concurrently interleaved: Java's lines must be a sub-multiset of ours
          const pool = g.stdout.split('\n');
          for (const line of j.stdout.split('\n').filter(Boolean)) {
            const at = pool.indexOf(line);
            assert.ok(at >= 0, `Java printed a line the browser runtime did not: ${JSON.stringify(line)}`);
            pool.splice(at, 1);
          }
        } else if (j.race) {
          assert.ok(g.stdout.startsWith(j.stdout), `Java stdout (truncated by its shutdown race) is not a prefix: ${JSON.stringify(j.stdout)}`);
        } else {
          assert.equal(j.status, Number(BigInt.asUintN(8, g.value)), 'Java exit status agrees');
          if (header(f.source, 'stdout-order')[0] === 'unordered') {
            // several actors print concurrently: only the multiset of lines is defined
            const lines = t => t.split('\n').sort().join('\n');
            assert.equal(lines(j.stdout), lines(g.stdout), 'Java stdout lines agree (any actor interleaving)');
          } else if (claim === 'diverges') {
            // a documented semantic divergence (see docs/DIFFERENTIAL.md): Java's
            // own output is pinned so drift on either side is caught
            const jw = header(f.source, 'java-stdout');
            assert.ok(jw.length, 'a "java: diverges" fixture pins Java output with "// java-stdout:" lines');
            assert.equal(j.stdout, jw.join('\n') + '\n', 'Java stdout matches the pinned divergent output');
          } else {
            assert.equal(j.stdout, g.stdout, 'Java stdout agrees');
          }
        }
      }
      javaChecked++;
    }
  });
}

for (const f of await fixtures('fail')) {
  check(`fail/${f.name}`, () => {
    const r = compile(f.source);
    assert.equal(r.ok, false, 'expected a compile error');
    assert.equal('wasm' in r, false);
    assert.equal(r.code, header(f.source, 'expect-error')[0], r.message);
    const line = header(f.source, 'expect-line')[0];
    if (line) assert.equal(r.line, Number(line), 'diagnostic line');
    assert.ok(r.line >= 1 && r.col >= 1, 'diagnostic has a position');
    if (attoc) {
      assert.equal(nativeBytes(f.path), null, 'native host also rejects');
      nativeChecked++;
    }
    if (java) {
      const claim = header(f.source, 'java')[0];
      const j = runJava(f.path);
      if (claim === 'rejects') assert.ok(j.rejected, `Java reference accepted it (exit ${j.status})`);
      else if (claim === 'accepts') assert.ok(!j.rejected, 'Java reference rejected it');
      else throw new Error('fixture lacks a "// java:" claim');
      javaChecked++;
    }
  });
}

for (const f of await fixtures('panic')) {
  check(`panic/${f.name}`, () => {
    const r = compile(f.source);
    assert.ok(r.ok, r.message);
    const g = runGuest(r.wasm);
    assert.equal(g.ok, false, 'expected a panic');
    assert.equal(g.panic, Number(header(f.source, 'expect-panic')[0]), 'panic code');
    const want = header(f.source, 'expect-stdout');
    assert.equal(g.stdout, want.length ? want.join('\n') + '\n' : '');
    if (attoc) { assert.deepEqual(nativeBytes(f.path), r.wasm); nativeChecked++; }
  });
}

// The binary name subsection must enumerate exactly the actual function indices.
// A stale checked_rem label previously shifted main's debug name after that
// helper was removed, even though all guest execution tests remained green.
check('wasm/name-section-matches-emitted-function-count', () => {
  for (const source of [
    'pub fnc main(): int { return 42; }',
    'pub fnc main(): int { stdio.println(42); return 42; }',
  ]) {
    const built = compile(source);
    assert.ok(built.ok, built.message);
    const mod = new WebAssembly.Module(built.wasm);
    const sections = WebAssembly.Module.customSections(mod, 'name');
    assert.equal(sections.length, 1, 'exactly one function name section');
    const bytes = new Uint8Array(sections[0]);
    let pos = 0;
    const leb = () => {
      let result = 0, shift = 0;
      for (let i = 0; i < 5; i++) {
        assert.ok(pos < bytes.length, 'LEB128 fits inside name section');
        const b = bytes[pos++];
        result += (b & 127) * 2 ** shift;
        if (!(b & 128)) return result;
        shift += 7;
      }
      throw new Error('invalid name section LEB128');
    };
    assert.equal(bytes[pos++], 1, 'function names subsection');
    const sectionLength = leb(), end = pos + sectionLength;
    assert.ok(end <= bytes.length, 'subsection stays in bounds');
    const count = leb(), names = [];
    for (let i = 0; i < count; i++) {
      const index = leb(), length = leb();
      assert.ok(pos + length <= end, 'name stays in subsection');
      names.push([index, decoder.decode(bytes.subarray(pos, pos + length))]);
      pos += length;
    }
    assert.equal(pos, end, 'name subsection decoded exactly');
    assert.equal(end, bytes.length, 'no unexpected second subsection');
    assert.deepEqual(names.map(([index]) => index), Array.from({length: count}, (_, i) => i));
    assert.equal(count, WebAssembly.Module.imports(mod).length + 4 + 1,
      'imports + four checked helpers + main');
    assert.deepEqual(names.slice(-5).map(([, name]) => name),
      ['atto.checked_add','atto.checked_sub','atto.checked_mul','atto.checked_neg','main']);
    assert.ok(!names.some(([, name]) => name === 'atto.checked_rem'),
      'removed remainder helper must not appear in guest debug names');
  }
});

// ---- generated programs: source-dependent artifacts --------------------------
check('generated/200-programs', () => {
  let seed = 12345;
  const rand = n => { seed = (seed * 1103515245 + 12345) % 2147483648; return seed % n; };
  const seen = new Set();
  for (let i = 0; i < 200; i++) {
    const a = rand(1000), b = rand(1000) + 1, c = rand(50);
    const src = `fnc f(int x, int y): int {\n  let acc = 0;\n  for (let i = 0; i < ${c}; i++) do\n    acc = acc + x + y;\n  done\n  return acc - y;\n}\npub fnc main(): int {\n  return f(${a}, ${b});\n}\n`;
    const r = compile(src);
    assert.ok(r.ok, r.message);
    seen.add(createHash('sha256').update(r.wasm).digest('hex'));
    assert.equal(runGuest(r.wasm).value, BigInt(c * (a + b) - b), src);
  }
  assert.ok(seen.size > 190, 'artifacts depend on source');
});

// The canonical AttoLang grammar has removed %, ^ and ^^: reject all
// three with a positioned diagnostic and no generated guest bytes.
for (const op of ['%', '^', '^^']) {
  check(`removed-operator/${op}`, () => {
    const r = compile(`pub fnc main(): int { return 5 ${op} 2; }`);
    assert.equal(r.ok, false);
    assert.equal(r.code, 'E_UNSUPPORTED');
    assert.equal('wasm' in r, false);
    assert.ok(r.line >= 1 && r.col >= 1);
  });
}

// ---- limits and robustness ---------------------------------------------------
check('limits/nesting', () => {
  const r = compile(`pub fnc main(): int { return ${'('.repeat(500)}1${')'.repeat(500)}; }`);
  assert.equal(r.code, 'E_NESTING');
});
// Left-associative chains and elif arms are parsed iteratively but walked
// recursively later; they share the 256-level tree-height budget.
const M = b => `pub fnc main(): int {\n${b}\n}`;
const chains = {
  add: n => M(`  return ${Array(n).fill('1').join(' + ')};`),
  mul: n => M(`  return ${Array(n).fill('1').join(' * ')};`),
  cmp: n => M(`  val b = ${Array(n).fill('1').join(' - ')} < 0;\n  return 0;`),
  eq: n => M(`  val b = ${Array(n).fill('true').join(' == ')};\n  return 0;`),
  and: n => M(`  val t = true;\n  if ${Array(n).fill('t').join(', ')} then\n    return 1;\n  fi\n  return 0;`),
  or: n => M(`  val t = false;\n  if ${Array(n).fill('t').join(' | ')} then\n    return 1;\n  fi\n  return 0;`),
  match: n => M(`  val x = ${n - 2};\n  match x\n${Array.from({ length: n - 1 }, (_, i) => `    ${i} -> { return ${i}; }\n`).join('')}    _ -> { return -1; }\n  end`),
  concat: n => M(`  val s = ${Array(n).fill('"ab"').join(' + ')};\n  return 0;`),
  float: n => M(`  val f = ${Array(n).fill('1.5').join(' * ')};\n  return 0;`),
  elif: n => M(`  val x = ${n - 1};\n  if x eq 0 then return 0;\n${Array.from({ length: n - 1 }, (_, i) => `  elif x eq ${i + 1} then return ${i + 1};\n`).join('')}  fi\n  return -1;`),
};
for (const [name, make] of Object.entries(chains)) {
  check(`limits/chain-${name}-within-budget`, () => {
    const r = compile(make(250));
    assert.equal(r.ok, true, r.message);
    runGuest(r.wasm);
  });
  check(`limits/chain-${name}-over-budget-fails-closed`, () => {
    assert.equal(compile(make(300)).code, 'E_NESTING');
  });
}
// Member and method links share the same budget; a receiver chain recurses
// through the checker and emitter once per link.
const CHAIN_CLASS = 'define class P as\n  pub int v: 1\n  pub n(): P { return new P(); }\nend\n';
const memberChains = {
  'method-chain-runs': n => CHAIN_CLASS + `pub fnc main(): int { return new P()${'.n()'.repeat(n - 1)}.v; }`,
  'member-chain-typed': n => `pub fnc main(): int { val a = 1; return a${'.b'.repeat(n)}; }`,
};
check('limits/method-chain-within-budget-executes', () => {
  const r = compile(memberChains['method-chain-runs'](240));
  assert.equal(r.ok, true, r.message);
  assert.equal(runGuest(r.wasm).value, 1n);
});
check('limits/member-chain-within-budget-is-a-diagnostic-not-a-trap', () => {
  assert.equal(compile(memberChains['member-chain-typed'](240)).code, 'E_TYPE');
});
for (const [name, make] of Object.entries(memberChains)) {
  check(`limits/${name}-over-budget-fails-closed`, () => {
    assert.equal(compile(make(300)).code, 'E_NESTING');
  });
}
check('limits/elif-chain-selects-the-right-arm', () => {
  assert.equal(runGuest(compile(chains.elif(250)).wasm).value, 249n);
});
check('robustness/adversarial-shapes-never-trap-compiler', () => {
  const huge = n => Array(n).fill('1');
  for (const src of [
    M(`  return ${huge(15000).join('+')};`),
    M(`  ${huge(15000).join('+')};\n  return 0;`),
    M(`  return ${'-'.repeat(30000)}1;`),
    M(`  return ${'- '.repeat(20000)}1;`),
    M(`  val b = ${'!'.repeat(30000)}true;\n  return 0;`),
    M(`  ${'block { '.repeat(5000)}${'} '.repeat(5000)}\n  return 0;`),
    M(`  ${'loop { '.repeat(5000)}`),
    M(`  return ${'f('.repeat(10000)}`),
    M(`  val t = true;\n  return ${'t ? 1 : '.repeat(5000)}0;`),
    M(`  if ${'('.repeat(10000)}`),
    M(Array.from({ length: 3000 }, (_, i) => `  val a${i} = ${i};`).join('\n')),
  ]) {
    const r = compile(src);
    if (r.ok) WebAssembly.validate(r.wasm);
  }
  assert.equal(runGuest(compile(M('  return 42;')).wasm).value, 42n, 'compiler still usable');
});
// Token-level mutation of every positive fixture. Mutants may loop forever, so
// accepted mutants are validated and instantiated but not executed.
{
  const pass = await fixtures('pass');
  const tokenize = s => s.match(/\s+|[A-Za-z_][A-Za-z0-9_]*|\d+|"(?:[^"\\\n]|\\.)*"|\S/g) ?? [];
  let seed = 12345;
  const rnd = n => (seed = (seed * 48271) % 2147483647) % n;
  let mutants = 0, accepted = 0;
  check('robustness/fixture-mutants-never-trap-compiler', () => {
    for (const f of pass) {
      const toks = tokenize(f.source);
      for (let k = 0; k < (Number(process.env.ATTO_FUZZ) || 60); k++) {
        const t = toks.slice();
        for (let edits = 1 + rnd(3); edits > 0; edits--) {
          const i = rnd(t.length), j = rnd(t.length);
          switch (rnd(4)) {
            case 0: t.splice(i, 1); break;
            case 1: t.splice(i, 0, t[j]); break;
            case 2: [t[i], t[j]] = [t[j], t[i]]; break;
            default: t[i] = ['(', ')', '{', '}', ';', ',', '|', 'fi', 'done', 'else', 'return', '-', '0', 'true', '"'][rnd(15)];
          }
        }
        mutants++;
        const r = compile(t.join(''));
        if (r.ok) {
          accepted++;
          assert.ok(WebAssembly.validate(r.wasm), 'accepted mutant produced invalid wasm');
        } else {
          assert.match(r.code, /^E_[A-Z_]+$/);
          assert.ok(r.line >= 1 && r.col >= 1, `diagnostic has a position: ${r.message}`);
        }
      }
    }
    assert.ok(mutants > 2000);
  });
  console.log(`mutation fuzz: ${mutants} mutants, ${accepted} accepted`);
}
check('limits/source-capacity', () => {
  const r = compile(`pub fnc main(): int { return 0; }${' '.repeat(api.input_capacity())}`);
  assert.equal(r.code, 'E_SOURCE_LIMIT');
});
check('robustness/recovery-after-error', () => {
  assert.equal(compile('pub fnc main(): int { return }').ok, false);
  const r = compile('pub fnc main(): int { return 42; }');
  assert.equal(runGuest(r.wasm).value, 42n);
});
check('robustness/random-garbage-never-traps-compiler', () => {
  let seed = 7;
  const alphabet = 'fnc main(){}[];:=+-*/%<>!|,.?"\'val let if then fi while do done 0123456789 abc\n\t';
  for (let i = 0; i < 2000; i++) {
    let s = '';
    const n = (seed = (seed * 48271) % 2147483647) % 120;
    for (let k = 0; k < n; k++) s += alphabet[(seed = (seed * 48271) % 2147483647) % alphabet.length];
    const r = compile(s);
    if (r.ok) runGuest(r.wasm);
  }
});

check('formal/compiled-guest-trace-nonvacuous', () => {
  assert.ok(tracedActorInstances >= 1, 'no compiled actor guest actually executed');
  assert.ok(tracedActorMessages >= 2, 'no compiled mailbox deliveries actually executed');
});
console.log('COMPILED_WASM_GUEST_TRACE actor_instances=' + tracedActorInstances +
  ' admitted_and_settled=' + tracedActorMessages +
  ' backend=browser-wasm guest=EXECUTED scheduling=serial-host-only');
console.log(digests.length ? `artifact digests:\n${digests.join('\n')}` : '');
// Exact current site preview corpus. Rejections never count as executing a
// feature; the report explicitly separates admitted, unsupported and guests.
try {
  const previewAudit = await auditPlaygroundPreviews({
    compile, runGuest, nativeBytes: attoc ? nativeBytes : null
  });
  console.log('PLAYGROUND_PREVIEW_AUDIT ' + JSON.stringify(previewAudit));
  passed++;
} catch (error) {
  failures.push('playground-preview-corpus: ' + error.message);
}

console.log(`conformance: ${passed} passed, ${failures.length} failed` +
  (java ? `, ${javaChecked} Java-differential` : ', Java differential skipped (set ATTO_JAVA)') +
  (attoc ? `, ${nativeChecked} native-vs-wasm` : ', native parity skipped (set ATTOC)'));
if (failures.length) {
  console.error(failures.join('\n'));
  process.exit(1);
}

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
import { readFile, readdir } from 'node:fs/promises';
import { execFileSync, spawnSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { join } from 'node:path';

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

// ---- guest runtime (atto.browser.guest.v1) ---------------------------------
const ALLOWED_IMPORTS = new Set(['print_i64', 'print_bool', 'print_str', 'print_newline']);
export function runGuest(wasm) {
  assert.ok(WebAssembly.validate(wasm), 'guest is valid wasm');
  const module = new WebAssembly.Module(wasm);
  for (const imp of WebAssembly.Module.imports(module)) {
    assert.equal(imp.module, 'atto');
    assert.ok(ALLOWED_IMPORTS.has(imp.name), `unexpected import ${imp.name}`);
  }
  const exportNames = WebAssembly.Module.exports(module).map(e => e.name).sort();
  assert.ok(['main', 'panic_code'].every(n => exportNames.includes(n)), 'guest exports main and panic_code');
  assert.ok(exportNames.every(n => ['main', 'panic_code', 'memory'].includes(n)), 'no extra exports');
  let out = '';
  let instance;
  const atto = {
    print_i64: v => { out += String(v); },
    print_bool: v => { out += v ? 'true' : 'false'; },
    print_str: (ptr, len) => { out += decoder.decode(new Uint8Array(instance.exports.memory.buffer, ptr, len)); },
    print_newline: () => { out += '\n'; },
  };
  instance = new WebAssembly.Instance(module, { atto });
  try {
    const value = instance.exports.main();
    assert.equal(typeof value, 'bigint');
    return { ok: true, value, stdout: out };
  } catch (error) {
    return { ok: false, panic: instance.exports.panic_code.value, error: String(error), stdout: out };
  }
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
function runJava(path) {
  const r = spawnSync(java, [path], { encoding: 'utf8', timeout: 60_000 });
  return { status: r.status, stdout: r.stdout, rejected: /oreslang: /.test(r.stderr + r.stdout) };
}
function nativeBytes(path) {
  const r = spawnSync(attoc, [path], { maxBuffer: 1 << 22 });
  return r.status === 0 ? new Uint8Array(r.stdout) : null;
}

let passed = 0, javaChecked = 0, nativeChecked = 0;
const failures = [];
function check(label, fn) {
  try { fn(); passed++; } catch (e) { failures.push(`${label}: ${e.message}`); }
}
const digests = [];

// This is a browser compiler/guest check, not Java reference parity:
// nominal definitions are admitted only while unreachable, with no guest
// actor scheduler, heap objects, or enum representation.
check('define/inert-declarations-guest-exec', () => {
  const source = [
    'define class Point as end',
    'define actor Worker as end',
    'define struct Pair as end',
    'define enum Status as',
    '  Pending',
    '  Ready',
    '  Failed',
    'end',
    'define pub module tools as end',
    'pub fnc main(): int { return 42; }',
  ].join('\n');
  const compiled = compile(source);
  assert.ok(compiled.ok, compiled.message);
  const guest = runGuest(compiled.wasm);
  assert.deepEqual([guest.ok, guest.value, guest.stdout], [true, 42n, '']);
  const plain = compile('pub fnc main(): int { return 42; }');
  assert.ok(plain.ok);
  assert.deepEqual(compiled.wasm, plain.wasm,
    'unreferenced declaration metadata must not change the executable Wasm');
});

check('define/rejection-and-no-artifact', () => {
  const main = 'pub fnc main(): int { return 0; }';
  for (const [prefix, code] of [
    ['define class P as\n fnc method(): int { return 0; }\nend\n', 'E_UNSUPPORTED'],
    ['define actor A as\n receive(int msg) -> void :: return; end\nend\n', 'E_UNSUPPORTED'],
    ['define enum E as X X end\n', 'E_SYNTAX'],
    ['define enum E as\n X\n X\nend\n', 'E_DUPLICATE'],
    ['define module Mod as\n fnc f(): int { return 2; }\nend\n', 'E_UNSUPPORTED'],
    ['define enum E as X end\ndefine class E as end\n', 'E_DUPLICATE'],
    ['define macro M as end\n', 'E_UNSUPPORTED'],
    ['define class C<T> as end\n', 'E_UNSUPPORTED'],
    ['define actor A as end\n', 'E_ENTRY'], // executable still needs main
  ]) {
    const result = compile(prefix + (code === 'E_ENTRY' ? '' : main));
    assert.equal(result.ok, false, prefix);
    assert.equal(result.code, code, result.message);
    assert.ok(result.line > 0 && result.col > 0, result.message);
    assert.equal('wasm' in result, false, prefix);
  }
  const spawn = compile('define actor A as end\npub fnc main(): int { val a = spawn A(); return 0; }');
  assert.equal(spawn.code, 'E_UNSUPPORTED');
  assert.equal('wasm' in spawn, false);
});


// Regression bridge from the earlier declaration/semantics stack:
// do not let unrelated valid class, enum and actor declarations mask removed
// operators or suppress eager right-hand-side guest effects.
const combinedDecls = [
  'define class Counter as',
  '  int start: 2',
  'end',
  'define enum Status as Ready, Done end',
  'define actor Worker as end',
].join('\n') + '\n';
for (const op of ['%', '^', '^^']) {
  check('combined-declarations/removed-operator-' + op, () => {
    const result = compile(combinedDecls +
      'pub fnc main(): int { val c = new Counter(); return c.start ' + op + ' 2; }');
    assert.equal(result.ok, false, 'removed operator accepted in declaration-bearing source');
    assert.equal(result.code, 'E_UNSUPPORTED', result.message);
    assert.equal('wasm' in result, false, 'invalid source must produce no Wasm guest');
    assert.ok(result.line > 0 && result.col > 0, 'diagnostic position must be valid');
  });
}
for (const [name, condition] of [
  ['and-false', 'false, crash()'],
  ['or-true', 'true | crash()'],
]) {
  check('combined-declarations/eager-' + name + '-traps', () => {
    const source = combinedDecls + [
      'fnc crash(): bool {',
      '  stdio.println("rhs");',
      '  val max_value = 9223372036854775807;',
      '  return max_value + 1 eq 0;',
      '}',
      'pub fnc main(): int {',
      '  val counter = new Counter();',
      '  val status: Status = Status.Ready;',
      '  if ' + condition + ' then',
      '    return counter.start;',
      '  fi',
      '  return 42;',
      '}',
    ].join('\n');
    const result = compile(source);
    assert.ok(result.ok, result.message);
    assert.deepEqual(compile(source).wasm, result.wasm, 'compilation must be deterministic');
    const guest = runGuest(result.wasm);
    assert.equal(guest.ok, false, 'eager RHS must trap rather than short-circuit');
    assert.equal(guest.panic, 1, 'checked-overflow panic code must survive');
    assert.equal(guest.stdout, 'rhs\n', 'RHS side effect must occur before trap');
  });
}

// Same source and compiler image runs on the native Rust host and wasm32.
// These tests execute the generated guest module, not just check parser AST.
check('enum/forward-type-guest-execution', () => {
  const src = [
    'fnc is_ready(Status value): bool { return value eq Status.Ready; }',
    'pub fnc main(): int {',
    '  val status: Status = Status.Ready;',
    '  if is_ready(status) then',
    '    return 42;',
    '  else',
    '    return 0;',
    '  fi',
    '}',
    'define enum Status as',
    '  Pending',
    '  Ready',
    '  Failed',
    'end',
  ].join('\n');
  const compiled = compile(src);
  assert.ok(compiled.ok, compiled.message);
  const result = runGuest(compiled.wasm);
  assert.deepEqual([result.ok, result.value, result.stdout], [true, 42n, '']);
  assert.deepEqual(compiled.wasm, compile(src).wasm, 'byte-identical deterministic compilation');
});

check('enum/function-and-reassignment', () => {
  const src = [
    'define enum Flag as Off, On end',
    'fnc flip(Flag f): Flag {',
    '  return f eq Flag.Off ? Flag.On : Flag.Off;',
    '}',
    'pub fnc main(): int {',
    '  let f: Flag = Flag.Off;',
    '  f = flip(f);',
    '  if f eq Flag.On then return 48; else return 0; fi',
    '}',
  ].join('\n');
  const compiled = compile(src);
  assert.ok(compiled.ok, compiled.message);
  assert.equal(runGuest(compiled.wasm).value, 48n);
});

check('enum/nominality-and-fail-closed', () => {
  const defs = 'define enum Flag as Off, On end\ndefine enum Mode as Off, On end\n';
  for (const [body, code] of [
    ['return Flag.Unknown;', 'E_UNKNOWN_VARIANT'],
    ['if Flag.On eq Mode.On then return 1; fi return 0;', 'E_TYPE'],
    ['if Flag.On eq 1 then return 1; fi return 0;', 'E_TYPE'],
    ['val n: int = Flag.Off; return n;', 'E_TYPE'],
    ['return Flag.On + 1;', 'E_TYPE'],
    ['val Flag = 1; return Flag;', 'E_DUPLICATE'],
    ['stdio.println(Flag.On); return 0;', 'E_UNSUPPORTED'],
    ['return Flag;', 'E_TYPE'],
  ]) {
    const r = compile(defs + 'pub fnc main(): int { ' + body + ' }');
    assert.equal(r.ok, false, body);
    assert.equal(r.code, code, r.message);
    assert.equal('wasm' in r, false, 'failed compile must not return a guest module');
  }
});

// Dedicated nominal-enum corpus is intentionally separate from the Java
// differential corpus: Java enum parity is not yet established.
// Fixed-size immutable-class subset. Actual native Rust and wasm32 compilers
// must produce identical guest code with a bounded, guest-private heap.
for (const f of await fixtures('class')) {
  check('class-fixture/' + f.name, () => {
    const r = compile(f.source);
    assert.ok(r.ok, r.message);
    assert.ok(WebAssembly.validate(r.wasm), 'generated guest is valid core Wasm');
    assert.deepEqual(compile(f.source).wasm, r.wasm, 'deterministic class layout');
    const g = runGuest(r.wasm);
    assert.ok(g.ok, g.error);
    assert.equal(g.value, BigInt(header(f.source, 'expect-exit')[0]));
    assert.equal(g.stdout, '');
    if (attoc) {
      assert.deepEqual(nativeBytes(f.path), r.wasm, 'native vs wasm32 class byte parity');
      nativeChecked++;
    }
  });
}

check('class/bounded-guest-heap', () => {
  const src = 'define class Unit as\n int value: 1\nend\n' +
    'pub fnc main(): int {\n for (let i = 0; i < 10000; i++) do\n' +
    '  val item = new Unit();\n done\n return 0;\n}';
  const compiled = compile(src);
  assert.ok(compiled.ok, compiled.message);
  const guest = runGuest(compiled.wasm);
  assert.equal(guest.ok, false, 'allocation beyond 64 KiB must trap');
  assert.equal(guest.panic, 3, 'bounded class allocator signals explicit heap exhaustion');
});

// Exact bump-pointer boundary: with no string data the heap begins at byte
// 16 in a 64-KiB non-growable page. An 8-byte object can be allocated
// (65536 - 16) / 8 = 8190 times, never 8191, without wrapping or overlapping.
check('class/heap-exact-capacity-and-per-instance-reset', () => {
  const sourceFor = count => 'define class Unit as\n int value: 1\nend\n' +
    'pub fnc main(): int {\n for (let i = 0; i < ' + count + '; i++) do\n' +
    '  val item = new Unit();\n done\n return 42;\n}';
  for (const [count,shouldTrap] of [[8190,false],[8191,true]]) {
    const r = compile(sourceFor(count));
    assert.ok(r.ok, r.message);
    for (let instance = 0; instance < 2; instance++) {
      // Each runGuest call instantiates a fresh Wasm module: the heap cursor
      // and panic_code must never leak across otherwise identical instances.
      const g = runGuest(r.wasm);
      assert.equal(g.ok, !shouldTrap, 'unexpected heap boundary at ' + count);
      if (shouldTrap) assert.equal(g.panic, 3, 'explicit bounded heap panic');
      else assert.equal(g.value, 42n, 'last permitted allocation succeeds');
    }
  }
});

// Multiple constructor results must survive evaluation of later arguments:
// the scratch allocator global must not replace an already pushed pointer.
check('class/multiple-constructor-arguments-and-literal-data-safety', () => {
  const src = [
    'define class Unit as',
    '  int value: 21',
    'end',
    'fnc sum(Unit a, Unit b): int { return a.value + b.value; }',
    'pub fnc main(): int {',
    '  stdio.println("class data intact");',
    '  return sum(new Unit(), new Unit());',
    '}',
  ].join('\n');
  const r = compile(src);
  assert.ok(r.ok, r.message);
  assert.deepEqual(compile(src).wasm, r.wasm, 'deterministic nested allocation');
  const g = runGuest(r.wasm);
  assert.ok(g.ok, g.error);
  assert.equal(g.value, 42n, 'two distinct constructor args remain readable');
  assert.equal(g.stdout, 'class data intact\n', 'heap must not clobber static string data');
});

check('class/field-access-and-ctor-negative-controls', () => {
  const prefix = 'define class P as\n int x: 7\n bool active: true\nend\n';
  for (const [body, code] of [
    ['val p = new P(); return p.missing;', 'E_UNKNOWN_FIELD'],
    ['val p = new P(1); return 0;', 'E_ARITY'],
    ['val p: int = new P(); return p;', 'E_TYPE'],
    ['return new P();', 'E_TYPE'],
    ['if new P() eq new P() then return 1; fi return 0;', 'E_UNSUPPORTED'],
    ['stdio.println(new P()); return 0;', 'E_UNSUPPORTED'],
    ['return new P().x + new P().active;', 'E_TYPE'],
    ['val p = P(); return 0;', 'E_NEW_REQUIRED'],
    ['return P().x;', 'E_NEW_REQUIRED'],
    ['return new Unknown().x;', 'E_NEW_TYPE'],
    ['return new P;', 'E_SYNTAX'],
    ['return new P(1).x;', 'E_ARITY'],
    ['val P = 2; return P;', 'E_DUPLICATE'],
  ]) {
    const compiled = compile(prefix + 'pub fnc main(): int { ' + body + ' }');
    assert.equal(compiled.ok, false, body);
    assert.equal(compiled.code, code, compiled.message);
    assert.equal('wasm' in compiled, false, 'rejected program must not emit bytes');
  }
});

check('class/exact-new-point-example-with-implicit-newlines', () => {
  const src = [
    'define class Point as',
    '  int x: 40',
    '  int y: 2',
    '  bool active: true',
    'end',
    '',
    'pub fnc main(): int {',
    '  val p = new Point()',
    '  if p.active then',
    '    return p.x + p.y',
    '  else',
    '    return 0',
    '  fi',
    '}',
  ].join('\n');
  const built = compile(src);
  assert.equal(built.ok, true, built.message);
  assert.ok(WebAssembly.validate(built.wasm));
  assert.equal(runGuest(built.wasm).value, 42n);
  const invalid = compile(src.replace('new Point()', 'Point()'));
  assert.equal(invalid.ok, false);
  assert.equal(invalid.code, 'E_NEW_REQUIRED');
  assert.equal('wasm' in invalid, false);
});

check('class/new-required-and-postfix-positive', () => {
  const src = 'define class P as\n int x: 42\nend\n' +
    'pub fnc main(): int { return (new P()).x; }';
  const result = compile(src);
  assert.equal(result.ok, true, result.message);
  assert.equal(runGuest(result.wasm).value, 42n);
  const rejected = compile(src.replace('new P()', 'P()'));
  assert.equal(rejected.ok, false);
  assert.equal(rejected.code, 'E_NEW_REQUIRED');
  assert.equal('wasm' in rejected, false);
});

for (const f of await fixtures('enum')) {
  check('enum-fixture/' + f.name, () => {
    const r = compile(f.source);
    assert.ok(r.ok, r.message);
    assert.deepEqual(compile(f.source).wasm, r.wasm, 'deterministic guest bytes');
    const g = runGuest(r.wasm);
    assert.ok(g.ok, g.error);
    assert.equal(g.value, BigInt(header(f.source, 'expect-exit')[0]));
    assert.equal(g.stdout, '');
    if (attoc) {
      assert.deepEqual(nativeBytes(f.path), r.wasm, 'native and wasm32 enum byte parity');
      nativeChecked++;
    }
  });
}

for (const f of await fixtures('pass')) {
  check(`pass/${f.name}`, () => {
    const r = compile(f.source);
    assert.ok(r.ok, `expected success, got ${r.message} at ${r.line}:${r.col}`);
    const again = compile(f.source);
    assert.deepEqual(again.wasm, r.wasm, 'deterministic output');
    digests.push(`${createHash('sha256').update(r.wasm).digest('hex')}  ${f.name}`);
    const g = runGuest(r.wasm);
    assert.ok(g.ok, `guest trapped: ${g.error}`);
    assert.equal(g.value, BigInt(header(f.source, 'expect-exit')[0]), 'main() result');
    const want = header(f.source, 'expect-stdout');
    assert.equal(g.stdout, want.length ? want.join('\n') + '\n' : '', 'stdout');
    if (attoc) {
      assert.deepEqual(nativeBytes(f.path), r.wasm, 'native and wasm32 compiler hosts emit identical bytes');
      nativeChecked++;
    }
    if (java) {
      const j = runJava(f.path);
      assert.ok(!j.rejected, `Java reference rejected it: ${j.stdout}`);
      assert.equal(j.status, Number(BigInt.asUintN(8, g.value)), 'Java exit status agrees');
      assert.equal(j.stdout, g.stdout, 'Java stdout agrees');
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
    assert.equal(count, WebAssembly.Module.imports(mod).length + 5 + 1,
      'imports + four checked arithmetic helpers + bounded class allocator + main');
    assert.deepEqual(names.slice(-6).map(([, name]) => name),
      ['atto.checked_add','atto.checked_sub','atto.checked_mul','atto.checked_neg','atto.class_alloc_bounded','main']);
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

// Browser-actor stage 1: the compiled guest really exports one checked receive
// handler. The host must create a separate worker/instance; this test only
// verifies the Wasm entrypoint and fail-closed compiler contracts.
check('actor/spawn-import-and-receive-guest',()=>{
  const source=[
    'define actor Worker as',
    '  receive(msg: int) -> int { return msg + 1; }',
    'end',
    'pub fnc main(): int {',
    '  val worker: Worker = spawn Worker();',
    '  return 0;',
    '}',
  ].join('\n');
  const r=compile(source);
  assert.ok(r.ok,r.message);
  const module=new WebAssembly.Module(r.wasm);
  assert.ok(WebAssembly.Module.imports(module).some(i=>i.module==='atto'&&i.name==='actor_spawn'));
  assert.ok(WebAssembly.Module.exports(module).some(i=>i.name==='atto_actor_receive_0'));
  let spawned=-1;
  const instance=new WebAssembly.Instance(module,{atto:{actor_spawn:id=>(spawned=id,1)}});
  assert.equal(instance.exports.main(),0n);
  assert.equal(spawned,0);
  assert.equal(instance.exports.atto_actor_receive_0(41n),42n);
});
check('actor/fail-closed-ownership-and-handler',()=>{
  for(const [src,expected] of [
    ['define actor Worker as end\npub fnc main(): int { val w = spawn Worker(); return 0; }','E_UNSUPPORTED'],
    ['define class Point as int x: 2 end\npub fnc main(): int { val w = spawn Point(); return 0; }','E_SPAWN_TYPE'],
    ['define actor Worker as receive(msg: int) -> int { return msg; } end\npub fnc main(): int { val w = spawn Worker(1); return 0; }','E_ARITY'],
    ['define actor Worker as receive(msg: int) -> int { return msg; } end\ndefine class Point as int x: 1 end\npub fnc main(): int { val w = spawn Worker(); w.send(new Point()); return 0; }','E_TYPE'],
    ['define actor Worker as receive(msg: int) -> int { return msg; } end\npub fnc main(): int { rt take w; return 0; }','E_UNSUPPORTED'],
    ['define actor Worker as receive(msg: int) -> int { return msg; } end\npub fnc main(): int { val w = spawn Worker(); return w; }','E_TYPE'],
    ['define actor Worker as receive(msg: int) -> int { stdio.println(msg); return msg; } end\npub fnc main(): int { val w = spawn Worker(); return 0; }','E_UNSUPPORTED'],
    ['define actor Worker as receive(msg: int) -> int { val nested = spawn Worker(); return msg; } end\npub fnc main(): int { val w = spawn Worker(); return 0; }','E_UNSUPPORTED'],
  ]){
    const r=compile(src);
    assert.equal(r.ok,false,src);
    assert.equal(r.code,expected,r.message);
    assert.equal('wasm' in r,false,'failed actor program must never emit guest bytes');
  }
});


check('actor/send-result-and-closed-admission-guest',()=>{
  const source=[
    'define actor Worker as',
    '  receive(msg: int) -> int { return msg + 1; }',
    'end',
    'pub fnc main(): int {',
    '  val w = spawn Worker();',
    '  val a: Result<void, SendError> = w.send(41);',
    '  val b = w.send(42);',
    '  val full = w.send(43);',
    '  val closed = w.close();',
    '  val after = w.send(44);',
    '  if a.is_ok() , b.is_ok() , full.is_full() , closed , after.is_closed() , after.is_err() then',
    '    return 42;',
    '  else',
    '    return 0;',
    '  fi',
    '}',
  ].join('\n');
  const result=compile(source);
  assert.ok(result.ok,result.message);
  const module=new WebAssembly.Module(result.wasm);
  const imports=WebAssembly.Module.imports(module).map(i=>i.name);
  assert.ok(imports.includes('actor_spawn'));
  assert.ok(imports.includes('actor_send'));
  assert.ok(imports.includes('actor_close'));
  const seen=[];let closed=false;
  const atto={
    actor_spawn:id=>{assert.equal(id,0);return 1},
    actor_send:(handle,value)=>{
      assert.equal(handle,1);assert.equal(typeof value,'bigint');
      if(closed)return 2;
      if(seen.length===2)return 1;
      seen.push(value);return 0;
    },
    actor_close:handle=>{assert.equal(handle,1);closed=true;return 1},
  };
  const instance=new WebAssembly.Instance(module,{atto});
  assert.equal(instance.exports.main(),42n);
  assert.deepEqual(seen,[41n,42n]);
});
check('actor/send-fails-closed-on-non-scalar-transfer',()=>{
  const prefix=[
    'define actor Worker as',
    '  receive(msg: int) -> int { return msg; }',
    'end',
    'define class Point as',
    '  int x: 1',
    'end',
  ].join('\n')+'\n';
  for(const statement of [
    'val w = spawn Worker(); val obj = new Point(); val result = w.send(obj);',
    'val w = spawn Worker(); val other = spawn Worker(); val result = w.send(other);',
    'val w = spawn Worker(); val result = w.send(true);',
    'val w = spawn Worker(); val result = w.send();',
    'val w = spawn Worker(); val result = w.send(1,2);',
    'val w = spawn Worker(); val result: Result<int, SendError> = w.send(1);',
  ]){
    const response=compile(prefix+'pub fnc main(): int { '+statement+' return 0; }');
    assert.equal(response.ok,false,statement);
    assert.equal('wasm' in response,false,'invalid send must never create guest code');
  }
});

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

console.log(digests.length ? `artifact digests:\n${digests.join('\n')}` : '');
console.log(`conformance: ${passed} passed, ${failures.length} failed` +
  (java ? `, ${javaChecked} Java-differential` : ', Java differential skipped (set ATTO_JAVA)') +
  (attoc ? `, ${nativeChecked} native-vs-wasm` : ', native parity skipped (set ATTOC)'));
if (failures.length) {
  console.error(failures.join('\n'));
  process.exit(1);
}

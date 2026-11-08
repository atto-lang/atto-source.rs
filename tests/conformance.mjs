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
    ['define actor A as\n receive(int msg): void { return; }\nend\n', 'E_UNSUPPORTED'],
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

// Actor receive, synthetic main and metered public wrapper consume three slots.
// Test the exact 1024-function boundary through the WASM compiler ABI.
check('actor/function-budget-includes-receive-and-synthetic-main', () => {
  const helper = count => Array.from({length:count},(_,i)=>
    'fnc helper_' + i + '(): int { return 0; }').join('\n') + '\n';
  const actor = 'define actor Worker as receive(int value): int { return value + 1; } end';
  const attempt = source => {
    const bytes = encoder.encode(source);
    assert.ok(bytes.length <= api.input_capacity(), 'quota test fits source input');
    new Uint8Array(api.memory.buffer,api.input_ptr(),bytes.length).set(bytes);
    return api.compile_actor(bytes.length);
  };
  assert.equal(attempt(helper(1022)+actor),1,'1025 total functions with wrapper must fail');
  assert.equal(api.result_length(),0,'over-budget actor must not expose Wasm');
  const mem = new Uint8Array(api.memory.buffer);
  let end=api.error_ptr();
  while(end<mem.length&&mem[end]&&end-api.error_ptr()<512)end++;
  assert.match(decoder.decode(mem.subarray(api.error_ptr(),end)),/^E_SOURCE_LIMIT:/);
  assert.equal(attempt(helper(1021)+actor),0,'1024 total functions including wrapper must fit');
  const guest = new Uint8Array(api.memory.buffer,api.result_ptr(),api.result_length()).slice();
  const mod=new WebAssembly.Module(guest);
  assert.deepEqual(WebAssembly.Module.imports(mod),[]);
  assert.deepEqual(WebAssembly.Module.exports(mod).map(e=>e.name),['receive']);
  assert.equal(new WebAssembly.Instance(mod,{}).exports.receive(41n),42n);
});

// Exercise the actual wasm32-exported compiler ABI, rather than using the
// safe host wrapper which rejects oversized inputs before entering Wasm.
check('actor/ABI-negative-and-overcapacity-lengths-fail-without-artifact', () => {
  const readError = () => {
    const mem = new Uint8Array(api.memory.buffer);
    const start = api.error_ptr();
    let end = start;
    while (end < mem.length && end - start < 512 && mem[end]) end++;
    return decoder.decode(mem.subarray(start, end));
  };
  for (const length of [-1, api.input_capacity() + 1, 0x7fffffff]) {
    assert.equal(api.compile_actor(length), 1, 'invalid ABI length ' + length);
    assert.equal(api.result_length(), 0, 'invalid length must clear previous guest');
    assert.match(readError(), /^E_SOURCE_LIMIT:/);
    assert.equal(api.error_line_number(), 1);
    assert.equal(api.error_column_number(), 1);
  }
});

check('actor/ABI-invalid-UTF8-never-leaks-previous-Wasm-and-recovers', () => {
  const source = 'define actor Identity as receive(int x): int { return x; } end';
  const bytes = encoder.encode(source);
  const emit = () => {
    new Uint8Array(api.memory.buffer, api.input_ptr(), bytes.length).set(bytes);
    assert.equal(api.compile_actor(bytes.length), 0);
    const output = new Uint8Array(api.memory.buffer,api.result_ptr(),api.result_length()).slice();
    assert.ok(WebAssembly.validate(output));
    return output;
  };
  const before = emit();
  assert.equal(new WebAssembly.Instance(new WebAssembly.Module(before),{}).exports.receive(-1n),-1n);
  new Uint8Array(api.memory.buffer,api.input_ptr(),2).set([0xc3,0x28]);
  assert.equal(api.compile_actor(2),1,'invalid UTF-8 must not enter parser');
  assert.equal(api.result_length(),0,'invalid UTF-8 must clear previous guest');
  const mem = new Uint8Array(api.memory.buffer);
  const start = api.error_ptr();
  let end = start;
  while (end < mem.length && end - start < 512 && mem[end]) end++;
  assert.match(decoder.decode(mem.subarray(start,end)),/^E_UTF8:/);
  assert.ok(api.error_line_number()>0 && api.error_column_number()>0);
  const after = emit();
  assert.deepEqual(after,before,'recovered actor bytes must be deterministic');
  assert.equal(api.error_line_number(),0,'success clears stale actor diagnostics');
  assert.equal(api.error_column_number(),0);
});

check('actor/compiler-entrypoints-must-not-confuse-main-and-receive-guests', () => {
  const write = source => {
    const input = encoder.encode(source);
    new Uint8Array(api.memory.buffer,api.input_ptr(),input.length).set(input);
    return input.length;
  };
  const main = 'pub fnc main(): int { return 42; }';
  const actor = 'define actor Worker as receive(int x): int { return x; } end';
  assert.equal(api.compile(write(main)),0);
  assert.ok(api.result_length()>0);
  assert.equal(api.compile_actor(write(main)),1,'ordinary main must not become actor guest');
  assert.equal(api.result_length(),0,'no normal guest may leak after rejected actor compile');
  assert.equal(api.compile(write(actor)),1,'actor receive must not silently compile as main');
  assert.equal(api.result_length(),0,'rejected main cannot expose previous Wasm');
  assert.equal(api.compile_actor(write(actor)),0);
  const bytes=new Uint8Array(api.memory.buffer,api.result_ptr(),api.result_length()).slice();
  const mod=new WebAssembly.Module(bytes);
  assert.deepEqual(WebAssembly.Module.imports(mod),[]);
  assert.deepEqual(WebAssembly.Module.exports(mod).map(x=>x.name),['receive']);
  assert.equal(new WebAssembly.Instance(mod,{}).exports.receive(42n),42n);
});
// Each actor function entry, statement and loop header immediately consumes
// one private fuel unit. A separate countdown amortizes the slow path to 128
// charges. Only a new public message gets a fresh allotment; a Web Worker
// that observes a trap must permanently retire the actor instance.
check('actor/immediate-fuel-boundary-resets-per-message-and-traps-runaway-loop', () => {
  const src = [
    'define actor Worker as',
    '  receive(int value): int {',
    '    let i = 0;',
    '    while i < value do',
    '      i = i + 1;',
    '    done',
    '    return i;',
    '  }',
    'end',
  ].join('\n');
  const input=encoder.encode(src);
  new Uint8Array(api.memory.buffer,api.input_ptr(),input.length).set(input);
  assert.equal(api.compile_actor(input.length),0);
  const wasm=new Uint8Array(api.memory.buffer,api.result_ptr(),api.result_length()).slice();
  assert.ok(WebAssembly.validate(wasm));
  const mod=new WebAssembly.Module(wasm);
  assert.deepEqual(WebAssembly.Module.imports(mod),[]);
  assert.deepEqual(WebAssembly.Module.exports(mod).map(e=>e.name),['receive']);
  const actor=new WebAssembly.Instance(mod,{});
  // This compiled actor consumes: one function entry, one initial binding,
  // one while-statement entry, (loop condition + assignment) * N,
  // one final condition and one return = 2*N + 5 fuel charges.
  // Fuel 131072 denies before the 131072nd charged operation.
  assert.equal(actor.exports.receive(65533n),65533n,
    '131071 charges fit in the actor fuel budget');
  assert.equal(actor.exports.receive(65533n),65533n,
    'per-message fuel must reset at the public wrapper boundary');
  assert.throws(()=>actor.exports.receive(65534n),WebAssembly.RuntimeError,
    'the first exhausted unit must trap before its statement effects');
  // Trapped actor instances are terminal in the Web Worker. Fresh instances
  // retain independent quotas and cannot inherit the exhausted fuel global.
  const fresh=new WebAssembly.Instance(mod,{});
  assert.equal(fresh.exports.receive(42n),42n);
});


check('actor/continue-paths-and-straight-line-metering-actually-run-in-wasm', () => {
  const src=[
    'define actor Worker as',
    '  receive(int value): int {',
    '    let i = 0;',
    '    let visited = 0;',
    '    while i < value do',
    '      i = i + 1;',
    '      if i < 10 then continue; fi',
    '      visited = visited + 1;',
    '    done',
    '    return visited;',
    '  }',
    'end'
  ].join('\n');
  const input=encoder.encode(src);
  new Uint8Array(api.memory.buffer,api.input_ptr(),input.length).set(input);
  assert.equal(api.compile_actor(input.length),0,'real compiler must accept continue loop');
  const guest=new Uint8Array(api.memory.buffer,api.result_ptr(),api.result_length()).slice();
  assert.ok(WebAssembly.validate(guest),'the instrumented emitted Wasm must validate');
  const module=new WebAssembly.Module(guest);
  assert.deepEqual(WebAssembly.Module.imports(module),[]);
  assert.deepEqual(WebAssembly.Module.exports(module).map(e=>e.name),['receive']);
  const instance=new WebAssembly.Instance(module,{});
  assert.equal(instance.exports.receive(9n),0n);
  assert.equal(instance.exports.receive(42n),33n);
  assert.throws(()=>instance.exports.receive(100000n),WebAssembly.RuntimeError,
    'nonterminating or large continue loop must exhaust local fuel');
  const fresh=new WebAssembly.Instance(module,{});
  assert.equal(fresh.exports.receive(15n),6n);
});

check('actor/recursive-function-depth-bounded-independent-of-loop-fuel', () => {
  const src = [
    'fnc recurse(int n): int {',
    '  if n eq 0 then return 42; fi',
    '  return recurse(n - 1);',
    '}',
    'define actor Worker as receive(int value): int { return recurse(value); } end',
  ].join('\n');
  const input=encoder.encode(src);
  new Uint8Array(api.memory.buffer,api.input_ptr(),input.length).set(input);
  assert.equal(api.compile_actor(input.length),0);
  const wasm=new Uint8Array(api.memory.buffer,api.result_ptr(),api.result_length()).slice();
  assert.ok(WebAssembly.validate(wasm));
  const mod=new WebAssembly.Module(wasm);
  assert.deepEqual(WebAssembly.Module.imports(mod),[]);
  const actor=new WebAssembly.Instance(mod,{});
  assert.equal(actor.exports.receive(60n),42n);
  assert.equal(actor.exports.receive(60n),42n, 'call depth resets after successful return');
  assert.throws(()=>actor.exports.receive(400n),WebAssembly.RuntimeError,
    'deep recursion must trap from depth quota before native stack overflow');
  assert.equal(new WebAssembly.Instance(mod,{}).exports.receive(5n),42n);
});

// Compile identical actor sources through the native Rust CLI and the wasm32
// compiler ABI. Both must emit byte-identical import-free actor guests; the
// ordinary compile path must fail closed rather than quietly erasing receive.
for (const fixture of await fixtures('actor')) {
  check('actor/native-wasm-byte-parity/' + fixture.name, () => {
    const input = encoder.encode(fixture.source);
    assert.ok(input.length <= api.input_capacity());
    new Uint8Array(api.memory.buffer,api.input_ptr(),input.length).set(input);
    assert.equal(api.compile_actor(input.length),0,'wasm32 actor fixture must compile');
    const wasm=new Uint8Array(api.memory.buffer,api.result_ptr(),api.result_length()).slice();
    assert.ok(WebAssembly.validate(wasm));
    const module=new WebAssembly.Module(wasm);
    assert.deepEqual(WebAssembly.Module.imports(module),[]);
    assert.deepEqual(WebAssembly.Module.exports(module).map(e=>e.name),['receive']);
    const instance=new WebAssembly.Instance(module,{});
    assert.equal(instance.exports.receive(BigInt(header(fixture.source,'actor-input')[0])),
      BigInt(header(fixture.source,'actor-result')[0]));
    if(attoc) {
      const native=spawnSync(attoc,['--actor',fixture.path],{maxBuffer:1<<22});
      assert.equal(native.status,0,'native actor fixture failed: '+String(native.stderr));
      assert.deepEqual(new Uint8Array(native.stdout),wasm,
        'native Rust vs wasm32 actor emitted Wasm must be byte identical');
      const normal=spawnSync(attoc,[fixture.path],{maxBuffer:1<<22});
      assert.equal(normal.status,1,'normal compiler must not reinterpret actor as main');
      assert.equal(normal.stdout.length,0,'rejected actor cannot emit main guest');
      nativeChecked++;
    }
  });
}

// ---- source-compiled scalar actor receive guest ABI --------------------------
check('actor/source-compiled-receive-guest-executes', () => {
  assert.equal(typeof api.compile_actor, 'function', 'compiler must expose separate actor ABI entrypoint');
  const src = 'define actor Worker as receive(int value): int { return value + 2; } end';
  const bytes = encoder.encode(src);
  new Uint8Array(api.memory.buffer, api.input_ptr(), bytes.length).set(bytes);
  assert.equal(api.compile_actor(bytes.length), 0);
  const guest = new Uint8Array(api.memory.buffer, api.result_ptr(), api.result_length()).slice();
  assert.ok(WebAssembly.validate(guest));
  const mod = new WebAssembly.Module(guest);
  assert.deepEqual(WebAssembly.Module.imports(mod), []);
  assert.deepEqual(WebAssembly.Module.exports(mod).map(x => x.name), ['receive']);
  const actor = new WebAssembly.Instance(mod, {});
  assert.equal(actor.exports.receive(40n), 42n);
  assert.equal(actor.exports.receive(-1n), 1n);
});
// Validate the actual emitted Wasm *section surface*, not only its public
// exports: a private memory, table, start or data section would violate the
// browser actor v0 contract even if WebAssembly.Module.exports() hid it.
check('actor/guest-v0-has-no-hidden-capability-sections', () => {
  const input=encoder.encode('define actor A as receive(int x): int { return x + 1; } end');
  new Uint8Array(api.memory.buffer,api.input_ptr(),input.length).set(input);
  assert.equal(api.compile_actor(input.length),0);
  const bytes=new Uint8Array(api.memory.buffer,api.result_ptr(),api.result_length()).slice();
  assert.ok(WebAssembly.validate(bytes));
  const mod=new WebAssembly.Module(bytes);
  assert.deepEqual(WebAssembly.Module.imports(mod),[]);
  assert.deepEqual(WebAssembly.Module.exports(mod).map(e=>e.name),['receive']);
  const forbidden=new Set([2,4,5,8,9,11,12,13]); // imports/table/memory/start/element/data/tag
  const ids=[]; let at=8;
  while(at<bytes.length){
    const id=bytes[at++]; let size=0,done=false;
    for(let i=0;i<5;i++){
      assert.ok(at<bytes.length,'Wasm section header must be in bounds');
      const b=bytes[at++];
      size+=(b&127)*2**(7*i);
      if(!(b&128)){done=true;break}
    }
    assert.ok(done&&Number.isSafeInteger(size),'bounded LEB128 section size');
    assert.ok(at+size<=bytes.length,'section body must be in bounds');
    ids.push(id); at+=size;
  }
  assert.equal(at,bytes.length);
  assert.deepEqual(ids.filter(id=>forbidden.has(id)),[],
    'no hidden actor memory/table/start/import/data sections');
  assert.equal(new WebAssembly.Instance(mod,{}).exports.receive(41n),42n);
});

// A scalar actor has the SAME canonical eager condition semantics as ordinary
// AttoLang functions: even an apparently decisive LHS must evaluate the RHS.
// With the boundary input, the RHS checked addition traps instead of being
// incorrectly suppressed by short-circuit lowering.
for (const [label, condition, safeResult] of [
  ['false-comma', 'false, value + 1 eq 0', 9n],
  ['true-pipe', 'true | value + 1 eq 0', 7n],
]) {
  check('actor/eager-' + label + '-executes-checked-rhs', () => {
    const src = 'define actor Worker as\n' +
      '  receive(int value): int {\n' +
      '    if ' + condition + ' then\n' +
      '      return 7;\n' +
      '    fi\n' +
      '    return 9;\n' +
      '  }\n' +
      'end';
    const input = encoder.encode(src);
    new Uint8Array(api.memory.buffer, api.input_ptr(), input.length).set(input);
    assert.equal(api.compile_actor(input.length), 0, label + ' must compile');
    const guest = new Uint8Array(api.memory.buffer, api.result_ptr(), api.result_length()).slice();
    assert.ok(WebAssembly.validate(guest));
    const mod = new WebAssembly.Module(guest);
    assert.deepEqual(WebAssembly.Module.imports(mod), []);
    assert.deepEqual(WebAssembly.Module.exports(mod).map(x => x.name), ['receive']);
    const actor = new WebAssembly.Instance(mod, {});
    assert.equal(actor.exports.receive(0n), safeResult);
    assert.throws(() => actor.exports.receive(9223372036854775807n),
      WebAssembly.RuntimeError, 'eager RHS checked arithmetic must not be skipped');
  });
}

// Rejected syntax must produce NO actor artifact even following a successful
// compile_actor call; subsequent valid source must still compile normally.
for (const op of ['%', '^', '^^']) {
  check('actor/removed-operator-' + op + '-does-not-leak-previous-guest', () => {
    const emit = source => {
      const input = encoder.encode(source);
      new Uint8Array(api.memory.buffer, api.input_ptr(), input.length).set(input);
      return api.compile_actor(input.length);
    };
    const valid = 'define actor Worker as receive(int value): int { return value + 1; } end';
    assert.equal(emit(valid), 0);
    assert.ok(api.result_length() > 0);
    assert.equal(emit('define actor Worker as receive(int value): int { return value ' + op + ' 2; } end'), 1);
    assert.equal(api.result_length(), 0, 'rejected actor must not expose last good guest');
    const memory = new Uint8Array(api.memory.buffer);
    let offset = api.error_ptr(), limit = offset + 512;
    while (offset < memory.length && offset < limit && memory[offset]) offset++;
    const diagnostic = decoder.decode(memory.subarray(api.error_ptr(), offset));
    assert.ok(diagnostic.startsWith('E_UNSUPPORTED:'), diagnostic);
    assert.equal(emit(valid), 0, 'compiler must recover after rejected actor');
    const wasm = new Uint8Array(api.memory.buffer, api.result_ptr(), api.result_length()).slice();
    const actor = new WebAssembly.Instance(new WebAssembly.Module(wasm), {});
    assert.equal(actor.exports.receive(41n), 42n);
  });
}

// Actor guest arithmetic must inherit the same checked-i64 semantics as
// ordinary compiler guests. A trap is terminal to the containing Web Worker;
// it may never silently wrap and return a misleading Result/ack.
for (const [label, expression, safeInput, safeOutput, overflowInput] of [
  ['add', 'value + 1', 9223372036854775806n, 9223372036854775807n, 9223372036854775807n],
  ['sub', 'value - 1', -9223372036854775807n, -9223372036854775808n, -9223372036854775808n],
  ['mul', 'value * 2', 4611686018427387903n, 9223372036854775806n, 4611686018427387904n],
]) {
  check('actor/checked-i64-' + label + '-traps-on-overflow', () => {
    const src = 'define actor Worker as receive(int value): int { return ' + expression + '; } end';
    const input = encoder.encode(src);
    new Uint8Array(api.memory.buffer, api.input_ptr(), input.length).set(input);
    assert.equal(api.compile_actor(input.length), 0, 'actor compilation must succeed');
    const bytes = new Uint8Array(api.memory.buffer, api.result_ptr(), api.result_length()).slice();
    assert.ok(WebAssembly.validate(bytes));
    const mod = new WebAssembly.Module(bytes);
    assert.deepEqual(WebAssembly.Module.imports(mod), [], 'actor must remain host-import-free');
    assert.deepEqual(WebAssembly.Module.exports(mod).map(x => x.name), ['receive']);
    const actor = new WebAssembly.Instance(mod, {});
    assert.equal(actor.exports.receive(safeInput), safeOutput);
    assert.throws(() => actor.exports.receive(overflowInput), WebAssembly.RuntimeError,
      'checked actor arithmetic must trap rather than wrap');
  });
}
for (const [label, src, want] of [
  ['wrong-type', 'define actor A as receive(bool value): int { return 42; } end', 'E_UNSUPPORTED'],
  ['multi-handlers', 'define actor A as receive(int x): int { return x; } receive(int y): int { return y; } end', 'E_DUPLICATE'],
  ['no-handler', 'define actor A as end', 'E_ACTOR_ENTRY'],
  ['unsupported-body', 'define actor A as int state: 0 end', 'E_UNSUPPORTED'],
  ['stdio-import', 'define actor A as receive(int x): int { stdio.println(x); return x; } end', 'E_ACTOR_IMPORT'],
]) {
  check(`actor/${label}-fails-closed`, () => {
    const bytes = encoder.encode(src);
    new Uint8Array(api.memory.buffer, api.input_ptr(), bytes.length).set(bytes);
    assert.equal(api.compile_actor(bytes.length), 1);
    assert.equal(api.result_length(), 0);
    const mem = new Uint8Array(api.memory.buffer);
    let end = api.error_ptr();
    while (mem[end]) end++;
    assert.ok(decoder.decode(mem.subarray(api.error_ptr(), end)).startsWith(want + ':'), label);
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

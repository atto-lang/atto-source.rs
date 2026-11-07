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

// ---- top-level declaration-only browser subset --------------------------------
for (const [name, declaration] of [
  ['class', 'define class Point as end'],
  ['actor', 'define actor Worker as end'],
  ['public-actor', 'define pub actor Worker as end'],
  ['brace', 'pub define class Point {}'],
  ['enum', 'define enum Status as Ready, Running, Done end'],
  ['multiline-enum', 'define enum Status as\nReady\nRunning\nend'],
  ['combined', 'define class Point as end\ndefine actor Worker as end\ndefine enum S as Ready, Done end'],
]) {
  check(`define/${name}-executes-scalar-guest`, () => {
    const r = compile(declaration + '\npub fnc main(): int { return 42; }');
    assert.equal(r.ok, true, r.message);
    assert.equal(runGuest(r.wasm).value, 42n);
  });
}
for (const [name, declaration, code] of [
  ['field', 'define class Point as\nint x: 0\nend', 'E_UNSUPPORTED'],
  ['actor-method', 'define actor Worker as\nreceive(m) {}\nend', 'E_UNSUPPORTED'],
  ['type-duplicate', 'define class X as end\ndefine actor X as end', 'E_DUPLICATE'],
  ['func-collision', 'define class main as end', 'E_DUPLICATE'],
  ['variant-duplicate', 'define enum S as Ready, Ready end', 'E_DUPLICATE'],
  ['payload', 'define enum S as Ready = 1 end', 'E_UNSUPPORTED'],
  ['unclosed', 'define actor Worker as', 'E_EOF'],
  ['unknown-define', 'define module M as end', 'E_UNSUPPORTED'],
  ['runtime-enum', 'define enum S as Ready end\npub fnc main(): int { return S.Ready; }', 'E_UNSUPPORTED'],
  ['nested', 'pub fnc main(): int { define class X as end\nreturn 42; }', 'E_UNSUPPORTED'],
]) {
  check(`define/${name}-rejects-no-artifact`, () => {
    const src = declaration === 'define actor Worker as' || declaration.includes('fnc main()') ? declaration : declaration + '\n' + 'pub fnc main(): int { return 42; }';
    const r = compile(src);
    assert.equal(r.ok, false, name);
    assert.equal(r.code, code, r.message);
    assert.equal('wasm' in r, false);
  });
}

// ---- executable no-receiver class static methods --------------------------------
for (const [name, declaration, want] of [
  ['simple', 'define class Calculator as static fnc add(int a, int b): int { return a + b; } end', 42],
  ['two-methods', 'define class C as static fnc mul(int a, int b): int { return a * b; } static fnc plus(int a, int b): int { return a + b; } end', 42],
  ['brace', 'pub define class Calc { pub static fnc answer(): int { return 42; } }', 42],
]) {
  check(`class-static/${name}-guest-executes`, () => {
    const invocation = name === 'simple' ? 'Calculator.add(19, 23)'
      : name === 'two-methods' ? 'C.plus(C.mul(3, 13), 3)' : 'Calc.answer()';
    const r = compile(declaration + '\n' + `pub fnc main(): int { return ${invocation}; }`);
    assert.equal(r.ok, true, r.message);
    assert.equal(runGuest(r.wasm).value, BigInt(want));
  });
}
for (const [name, def, code] of [
  ['instance-method', 'define class C as fnc value(): int { return 42; } end', 'E_UNSUPPORTED'],
  ['constructor', 'define class C as constructor() {} end', 'E_UNSUPPORTED'],
  ['field', 'define class C as int answer: 42 end', 'E_UNSUPPORTED'],
  ['duplicate-static', 'define class C as static fnc f(): int { return 1; } static fnc f(): int { return 2; } end', 'E_DUPLICATE'],
  ['call-unknown', 'define class C as end\npub fnc main(): int { return C.f(); }', 'E_UNBOUND'],
]) {
  check(`class-static/${name}-rejects-no-wasm`, () => {
    const src = def.includes('fnc main()') ? def : def + '\n' + 'pub fnc main(): int { return 0; }';
    const r = compile(src);
    assert.equal(r.ok, false);
    assert.equal(r.code, code, r.message);
    assert.equal('wasm' in r, false);
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

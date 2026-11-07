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
    const src = `fnc f(int x, int y): int {\n  let acc = 0;\n  for (let i = 0; i < ${c}; i++) do\n    acc = acc + x % y;\n  done\n  return acc - y;\n}\npub fnc main(): int {\n  return f(${a}, ${b});\n}\n`;
    const r = compile(src);
    assert.ok(r.ok, r.message);
    seen.add(createHash('sha256').update(r.wasm).digest('hex'));
    assert.equal(runGuest(r.wasm).value, BigInt(c * (a % b) - b), src);
  }
  assert.ok(seen.size > 190, 'artifacts depend on source');
});

// ---- limits and robustness ---------------------------------------------------
check('limits/nesting', () => {
  const r = compile(`pub fnc main(): int { return ${'('.repeat(500)}1${')'.repeat(500)}; }`);
  assert.equal(r.code, 'E_NESTING');
});
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

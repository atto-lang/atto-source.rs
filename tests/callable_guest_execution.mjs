// Runs native-compiler-generated Wasm guests for all currently implemented callable forms.
// In particular, this checks *execution* rather than just compiler acceptance.
import assert from 'node:assert/strict';
import { readFile, writeFile, mkdtemp, rm, stat } from 'node:fs/promises';
import { spawnSync } from 'node:child_process';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';

const attoc = process.env.ATTOC || fileURLToPath(new URL('../target/debug/attoc', import.meta.url));
const fixtures = [
  '165-callable-five-function-forms.atto',
  '166-callable-five-routine-forms.atto',
  '167-callable-five-inline-method-forms.atto',
  '168-callable-end-nested-arg-commas.atto',
  '173-class-mtd-executable.atto',
];
const negative = [
  '39-end-form-body.atto',
  '165-callable-trailing-comma.atto',
  '166-callable-colon-colon-brace.atto',
  '167-callable-brace-comma.atto',
  '168-callable-nested-brace-comma.atto',
  '169-callable-colon-arrow-end.atto',
];
const allowed = new Set(['print_i64','print_bool','print_str','print_newline','print_f64','actor_spawn','actor_send']);
const imports = {
  atto: {
    print_i64: () => { throw Error('unexpected printing'); },
    print_bool: () => { throw Error('unexpected printing'); },
    print_str: () => { throw Error('unexpected printing'); },
    print_newline: () => { throw Error('unexpected printing'); },
    print_f64: () => { throw Error('unexpected printing'); },
    actor_spawn: () => { throw Error('unexpected actor spawn'); },
    actor_send: () => { throw Error('unexpected actor send'); },
  }
};
const run = (source, out) => spawnSync(attoc, [source, '-o', out], { encoding: 'utf8', timeout: 30_000, maxBuffer: 1 << 20 });
const work = await mkdtemp(join(tmpdir(), 'atto-callable-'));
try {
  for (const name of fixtures) {
    const path = fileURLToPath(new URL('./fixtures/pass/' + name, import.meta.url));
    const source = await readFile(path, 'utf8');
    const want = source.match(/^\/\/ expect-exit: (\d+)/m)?.[1];
    assert.ok(want, name + ': expected exit header missing');
    const out = join(work, name + '.wasm');
    const built = run(path, out);
    assert.equal(built.status, 0, name + ': attoc failed: ' + built.stderr);
    const wasm = await readFile(out);
    assert.ok(WebAssembly.validate(wasm), name + ': invalid guest');
    const module = new WebAssembly.Module(wasm);
    for (const imp of WebAssembly.Module.imports(module)) {
      assert.equal(imp.module, 'atto', name + ': wrong import module');
      assert.ok(allowed.has(imp.name), name + ': unexpected import ' + imp.name);
    }
    const guest = new WebAssembly.Instance(module, imports);
    const actual = guest.exports.main();
    assert.equal(actual, BigInt(want), name + ': wrong guest return value');
    assert.equal(guest.exports.panic_code.value, 0, name + ': unexpected panic');
    console.log('PASS guest execution', name, String(actual));
  }
  // Named-call separator equivalence is observed in real compiled Wasm.
  // Noncommutative result distinguishes parameter binding from source position.
  const prefix = 'fnc f(int a, int b): int { return a * 10 + b; } ';
  const scalarCases = [
    ['colon', 'f(b: 2, a: 4)', 42n],
    ['equals', 'f(b=2,a=4)', 42n],
    ['mixed', 'f(b:2,a=4)', 42n],
    ['mixed-reverse', 'f(b=2,a:4)', 42n],
    ['positional-named', 'f(4,b=2)', 42n],
    ['distinct-result', 'f(b=4,a=2)', 24n],
  ];
  for (const [id, call, expected] of scalarCases) {
    const source = join(work, 'named-' + id + '.atto');
    const out = source + '.wasm';
    await writeFile(source, prefix + 'pub fnc main(): int { return ' + call + '; }');
    const built = run(source, out);
    assert.equal(built.error, undefined, id + ': compiler infrastructure failure');
    assert.equal(built.signal, null, id + ': compiler signal');
    assert.equal(built.status, 0, id + ': ' + built.stderr);
    const bytes = await readFile(out);
    assert.ok(WebAssembly.validate(bytes), id + ': invalid Wasm');
    const module = new WebAssembly.Module(bytes);
    assert.deepEqual(WebAssembly.Module.imports(module), [], id + ': unexpected host imports');
    const guest = new WebAssembly.Instance(module, {});
    for (let i = 0; i < 2; i++) {
      assert.equal(guest.exports.main(), expected, id + ': wrong parameter-slot binding');
      assert.equal(guest.exports.panic_code.value, 0, id + ': panic');
    }
    console.log('PASS named argument guest', id);
  }
  // Observe both order and count of effects on an admitted in-order call.
  {
    const source = join(work, 'named-effects.atto'), out = source + '.wasm';
    await writeFile(source, prefix +
      'fnc tick(int v): int { stdio.println(v); return v; } ' +
      'pub fnc main(): int { return f(a=tick(4), b:tick(2)); }');
    const built = run(source, out);
    assert.equal(built.error, undefined);
    assert.equal(built.signal, null);
    assert.equal(built.status, 0, built.stderr);
    const module = new WebAssembly.Module(await readFile(out));
    assert.deepEqual(WebAssembly.Module.imports(module).map(x => x.module + '.' + x.name).sort(),
      ['atto.print_i64', 'atto.print_newline']);
    const events = [];
    const guest = new WebAssembly.Instance(module, {atto:{
      print_i64: v => { assert.equal(typeof v, 'bigint'); events.push(v); },
      print_newline: () => events.push('newline'),
    }});
    for (let i = 0; i < 2; i++) {
      assert.equal(guest.exports.main(), 42n);
      assert.equal(guest.exports.panic_code.value, 0);
    }
    assert.deepEqual(events, [4n,'newline',2n,'newline',4n,'newline',2n,'newline']);
    console.log('PASS named argument effects in source order');
  }
  for (const [id, call, diagnostic] of [
    ['duplicate-mixed', 'f(a=4,a:2)', 'E_DUPLICATE'],
    ['duplicate-positional', 'f(4,a=2)', 'E_DUPLICATE'],
    ['unknown', 'f(z=4,b:2)', 'E_NAMED_ARG'],
    ['positional-after-named', 'f(a=4,2)', 'E_SYNTAX'],
    ['reorder-effect', 'f(b=tick(2),a:4)', 'E_EFFECT_ORDER'],
  ]) {
    const source = join(work, 'named-reject-' + id + '.atto'), out = source + '.wasm';
    await writeFile(source, prefix + 'fnc tick(int v): int { stdio.println(v); return v; } ' +
      'pub fnc main(): int { return ' + call + '; }');
    const built = run(source, out);
    assert.equal(built.error, undefined, id + ': compiler infrastructure failure');
    assert.equal(built.signal, null, id + ': compiler signal');
    assert.equal(built.status, 1, id + ': expected rejection');
    assert.match(built.stderr, new RegExp('\\b' + diagnostic + '\\b'), id + ': wrong diagnostic');
    await assert.rejects(stat(out), {code:'ENOENT'}, id + ': rejected source emitted artifact');
    console.log('PASS named argument rejection', id);
  }
  for (const name of negative) {
    const path = fileURLToPath(new URL('./fixtures/fail/' + name, import.meta.url));
    const source = await readFile(path, 'utf8');
    const want = source.match(/^\/\/ expect-error: (E_\w+)/m)?.[1];
    assert.ok(want, name + ': expected error header missing');
    const output = join(work, name + '.wasm');
    const built = run(path, output);
    assert.equal(built.status, 1, name + ': expected compiler rejection');
    assert.ok(built.stderr.includes(want), name + ': missing ' + want + ': ' + built.stderr);
    await assert.rejects(stat(output), {code:'ENOENT'}, name + ': rejected source emitted an artifact');
    console.log('PASS expected diagnostic', name, want);
  }
} finally {
  await rm(work, { recursive: true, force: true });
}

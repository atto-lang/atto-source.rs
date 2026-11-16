// End-to-end negative and positive type-casting checks on actual Atto guest Wasm.
// This proves only statically monomorphic scalar Any, NOT general dynamic casts.
import assert from 'node:assert/strict';
import {mkdtemp, readFile, rm, stat, writeFile} from 'node:fs/promises';
import {spawnSync} from 'node:child_process';
import {tmpdir} from 'node:os';
import {join} from 'node:path';

const attoc = process.env.ATTOC || 'target/release/attoc';
const program = body => 'fnc f(Any v): int { ' + body +
  ' } pub fnc main(): int { val value: Any = 41; return f(value); }';
const pass = [
  ['capture-when', program('match v over on is int as val n when n > 40 -> { return n + 1; } on is int -> { return 0; } on _ -> { return -1; } end')],
  ['negated-capture', program('match v over on is !string as val n when n > 40 -> { return n + 1; } on _ -> { return 0; } end')],
  ['negated-is-expression', program('if v is !string && v == 41 then return v + 1; fi return 0;')],
  ['guard-fallback', program('match v over on is int as val n when n > 90 -> { return 0; } on is int as val m -> { return m + 1; } on _ -> { return -1; } end')],
  ['negated-nonmatch', program('match v over on is !int -> { return 0; } on is int as val n -> { return n + 1; } end')],
  ['mismatch-else', program('if v is string then return 0; fi return 42;')]
];
const fail = [
  ['missing-capture-qualifier', 'E_SYNTAX', program('match v over on is int as n -> { return n; } on _ -> { return 0; } end')],
  ['mutable-capture-not-proven', 'E_SYNTAX', program('match v over on is int as let n -> { return n; } on _ -> { return 0; } end')],
  ['unknown-runtime-type', 'E_UNSUPPORTED_TYPE', program('match v over on is Widget -> { return 0; } on _ -> { return 1; } end')],
  ['no-dynamic-any', 'E_UNSUPPORTED_TYPE', 'fnc f(Any v): int { return 42; } pub fnc main(): int { val value: Any = 40 + 1; return f(value); }'],
  ['using-is-not-cast', 'E_UNSUPPORTED', 'pub fnc main(): int { using 41 as r { return 42; } return 0; }'],
  ['as-type-cast-not-admitted', 'E_UNSUPPORTED', program('return v as int;')]
];
const noIo = () => { throw Error('unexpected host capability'); };
const imports = {atto: {
  print_i64: noIo, print_bool: noIo, print_str: noIo,
  print_newline: noIo, print_f64: noIo, actor_spawn: noIo, actor_send: noIo
}};
const work = await mkdtemp(join(tmpdir(), 'atto-type-casts-'));
try {
  for (const [name, source] of pass) {
    const src = join(work, name + '.atto');
    const output = join(work, name + '.wasm');
    await writeFile(src, source);
    const built = spawnSync(attoc, [src, '-o', output], {encoding:'utf8', timeout:30000});
    assert.equal(built.status, 0, name + ': ' + built.stderr);
    const bytes = await readFile(output);
    assert.ok(WebAssembly.validate(bytes), name + ': invalid guest Wasm');
    const mod = new WebAssembly.Module(bytes);
    for (const imp of WebAssembly.Module.imports(mod)) {
      assert.equal(imp.module, 'atto', name + ': wrong import module');
      assert.ok(Object.hasOwn(imports.atto, imp.name), name + ': unexpected host import');
    }
    const guest = new WebAssembly.Instance(mod, imports);
    assert.equal(guest.exports.main(), 42n, name + ': incorrect guest result');
    assert.equal(guest.exports.panic_code.value, 0, name + ': unexpected panic');
    console.log('PASS guest execution', name);
  }
  for (const [name, code, source] of fail) {
    const src = join(work, name + '.atto');
    const output = join(work, name + '.wasm');
    await writeFile(src, source);
    const built = spawnSync(attoc, [src, '-o', output], {encoding:'utf8', timeout:30000});
    assert.equal(built.status, 1, name + ': invalid code compiled');
    assert.ok(built.stderr.includes(code), name + ': expected ' + code + ', got ' + built.stderr);
    await assert.rejects(stat(output), {code:'ENOENT'}, name + ': rejection emitted an artifact');
    console.log('PASS fail-closed rejection', name, code);
  }
} finally {
  await rm(work, {recursive:true, force:true});
}

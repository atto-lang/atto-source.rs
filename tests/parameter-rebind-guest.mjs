// Execute actual compiled guests; no mock compiler or success on skip.
import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { mkdtempSync, writeFileSync, readFileSync, rmSync, existsSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { spawnSync } from 'node:child_process';
const compiler = resolve(process.env.ATTOC || 'target/release/attoc');
const dir = mkdtempSync(join(tmpdir(), 'atto-rebind-'));
function build(source, explicit = false) {
  const input = join(dir, 'main.atto');
  const output = join(dir, 'main.wasm');
  writeFileSync(input, source);
  rmSync(output, { force: true });
  const r = spawnSync(compiler, [...(explicit ? ['--explicit-rebinding'] : []), 'main.atto', '-o', output], {
    cwd: dir, encoding: 'utf8', timeout: 30000
  });
  assert.ifError(r.error);
  return { r, output };
}
function guest(source, expected, explicit = false) {
  const { r, output } = build(source, explicit);
  assert.equal(r.status, 0, r.stderr);
  const bytes = readFileSync(output);
  assert.ok(WebAssembly.validate(bytes));
  const module = new WebAssembly.Module(bytes);
  assert.deepEqual(WebAssembly.Module.imports(module), []);
  const instance = new WebAssembly.Instance(module, {});
  assert.equal(instance.exports.main(), BigInt(expected));
  assert.equal(instance.exports.panic_code.value, 0);
  return bytes;
}
function reject(source, code, explicit = false) {
  const { r, output } = build(source, explicit);
  assert.equal(r.status, 1, source + ': ' + r.stderr);
  assert.match(r.stderr, new RegExp(code));
  assert.equal(existsSync(output), false);
}
try {
  // Caller retains its original scalar; only the local formal changes.
  const main = 'pub fnc main(): int { val int n = 20; return bump(n) + n; }';
  guest('fnc bump(rebind int n): int { n = n + 2; return n; } ' + main, 42);
  guest('fnc bump(rebind n: int): int { n = n + 2; return n; } ' + main, 42);
  reject('fnc bump(int n): int { n = n + 2; return n; } ' + main, 'E_IMMUTABLE');
  const fixed = 'fnc id(int n): int { return n; } pub fnc main(): int { return id(42); }';
  assert.deepEqual(guest(fixed, 42), guest(fixed.replace('int n', 'rebind int n'), 42));
  reject('fnc replace(rebind mut Foo f): void {} pub fnc main(): int { return 0; }', 'E_UNSUPPORTED');
  guest('pub fnc main(): int { let int x = 39; set x = x + 1; set! x = x + 1; set! (x = x + 1); return x; }', 42);
  guest('pub fnc main(): int { let x = 40; let x = x + 1; set x = x + 1; return x; }', 42);
  reject('pub fnc main(): int { const x = 40; set! x = 42; return x; }', 'E_IMMUTABLE');
  reject('pub fnc main(): int { const x = 40; const x = 42; return x; }', 'E_DUPLICATE');
  guest('pub fnc main(): int { let x = 0; for (set x = 40; x < 42; set! (x = x + 1)) {} return x; }', 42, true);
  guest('pub fnc main(): int { let x = 40; set x = x + 2; return x; }', 42, true);
  reject('pub fnc main(): int { let x = 40; x = 42; return x; }', 'E_EXPLICIT_REBIND', true);
  reject('pub fnc main(): int { let x = 0; for (x = 40; x < 42; x++) {} return x; }', 'E_EXPLICIT_REBIND', true);
  reject('pub fnc main(): int { let x = 40; for (; x < 42; x = x + 1) {} return x; }', 'E_EXPLICIT_REBIND', true);
  guest('pub fnc main(): int { let x = 40; set x++; set ++x; return x; }', 42, true);
  guest('pub fnc main(): int { let x = 44; set x--; set --x; return x; }', 42, true);
  guest('pub fnc main(): int { let x = 20; set x += 2; set x -= 1; set x *= 2; return x; }', 42, true);
  // The existing signed-division proof permits nonzero literal divisors.
  // Compound updates must use precisely that proof, not a new trap policy.
  guest('pub fnc main(): int { let x = 85; set x /= 2; return x; }', 42, true);
  guest('pub fnc main(): int { let x = -85; set x /= 2; return 84 + x; }', 42, true);
  reject('pub fnc main(): int { let x = 84; set x /= 0; return x; }', 'E_UNSUPPORTED', true);
  reject('pub fnc main(): int { let x = -9223372036854775808; set x /= -1; return x; }', 'E_UNSUPPORTED', true);
  reject('fnc divide(int d): int { let x = 84; set x /= d; return x; } pub fnc main(): int { return divide(2); }', 'E_UNSUPPORTED', true);
  guest('pub fnc main(): int { let x = 32; for let i = 0; i < 10; set i++ { set x++; } return x; }', 42, true);
  guest('pub fnc main(): int { let x = 32; for (let i = 10; i > 0; set --i) { set x++; } return x; }', 42, true);
  guest('pub fnc main(): int { let x = 32; for let i = 0; i < 10; set i += 2 { set x += 2; } return x; }', 42, true);
  reject('pub fnc main(): int { let x = 40; x += 2; return x; }', 'E_EXPLICIT_REBIND', true);
  reject('pub fnc main(): int { for let i = 0; i < 10; i++ {} return 42; }', 'E_EXPLICIT_REBIND', true);
  reject('pub fnc main(): int { const x = 40; set x++; return x; }', 'E_IMMUTABLE', true);
  guest("pub fnc main(): int { return match 42 over on _ as int z -> z end; }", 42);
  guest("pub fnc main(): int { return match 42 over on _ as const z -> z end; }", 42);
  guest("pub fnc main(): int { let int n = 20; let int r = 0; do match n over on _ as rebind int z -> { z = z + 2; r = z; } end return n + r; }", 42);
  guest("pub fnc main(): int { return match 42 over on 42 as rebind z when z eq 42 -> { z = z + 1; return z; } on _ -> 0 end; }", 43);
  guest("pub fnc main(): int { return match 42 over on 41 as z when z eq 42 -> 0, on _ as const z -> z end; }", 42);
  reject("pub fnc main(): int { do match 42 over on _ as int z -> { z = 0; } end return 0; }", "E_IMMUTABLE");
  reject("pub fnc main(): int { return match 42 over on _ as bool z -> 0 end; }", "E_TYPE");
  reject("pub fnc main(): int { return match 42 over on _ as rebind rebind int z -> z end; }", "E_SYNTAX");
  reject("pub fnc main(): int { return match 42 over on _ as mut z -> 0 end; }", "E_UNSUPPORTED");
  reject("pub fnc main(): int { do match 42 over on _ as rebind z -> {} end return z; }", "E_UNBOUND");
  // Pin the original conformance source bytes before compiler execution.
  for (const [name, blob, expected, diagnostic] of [["incoming-alias-typed","a468f12a990b9e228b3dda649ba839526d3a32f9",42,null],["incoming-alias-rebind","389e63ac3aea25b4f20d2baac14908d98809b57a",42,null],["incoming-alias-fixed-reject","3f8fbcd373405d950a16bcc2779d78d46027f493",null,"E_IMMUTABLE"],["incoming-alias-type-reject","f92cd4e128cafdface1a1e0eb8abd3cb0d14698f",null,"E_TYPE"],["incoming-alias-duplicate-reject","c35ceb04f96e8aab70efded4d6ba09854167a390",null,"E_SYNTAX"],["scalar-name-first","acd7353fb5b0d17e48dffad206210e770304a2f8",23,null]]) {
    const bytes = readFileSync(resolve('contracts/incoming-scalar-bindings-v1', name + '.atto'));
    const actual = createHash('sha1').update(Buffer.from('blob ' + bytes.length + '\0')).update(bytes).digest('hex');
    assert.equal(actual, blob, name + ': shared source identity');
    if (expected === null) reject(bytes.toString('utf8'), diagnostic);
    else guest(bytes.toString('utf8'), expected);
  }
  console.log('scalar rebind and match alias guest cases passed');
} finally {
  rmSync(dir, { recursive: true, force: true });
}

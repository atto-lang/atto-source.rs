// Exact-source compiled guest oracle for the restricted field=literal struct
// initializer. This is NOT a host model of struct behavior. Requires a real
// native attoc built from the same checked-out commit: ATTOC=/abs/path/attoc
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { mkdtempSync, readFileSync, writeFileSync, rmSync, existsSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { resolve, join, isAbsolute } from 'node:path';

const compiler = process.env.ATTOC;
if (!compiler || !isAbsolute(compiler) || !existsSync(compiler)) {
  throw new Error('ATTOC must identify the executable native compiler by absolute path; no skip');
}
const dir = mkdtempSync(join(tmpdir(), 'atto-struct-equals-'));
const header = 'define struct Particle as int x; int y; end\n';
function compile(source, label) {
  const sourcePath = join(dir, `${label}.atto`);
  const outPath = join(dir, `${label}.wasm`);
  writeFileSync(sourcePath, source);
  const result = spawnSync(compiler, [sourcePath, '-o', outPath],
    { encoding: 'utf8', timeout: 30000, maxBuffer: 1024 * 1024 });
  if (result.error) throw result.error;
  return { result, outPath, bytes: existsSync(outPath) ? readFileSync(outPath) : null };
}
function instantiate(wasm) {
  assert.equal(WebAssembly.validate(wasm), true, 'compiler must produce valid guest Wasm');
  const module = new WebAssembly.Module(wasm);
  const atto = {
    print_i64: () => {}, print_bool: () => {}, print_str: () => {},
    print_newline: () => {}, print_f64: () => {},
    actor_spawn: () => { throw new Error('unexpected actor spawn'); },
    actor_send: () => { throw new Error('unexpected actor send'); },
    state_register: () => { throw new Error('unexpected state'); },
    state_enter: () => { throw new Error('unexpected state'); },
    state_leave: () => { throw new Error('unexpected state'); },
  };
  for (const imp of WebAssembly.Module.imports(module)) {
    assert.equal(imp.module, 'atto', 'no unreviewed guest imports');
    assert.equal(imp.kind, 'function', 'no unexpected memory/table/global imports');
    assert.equal(typeof atto[imp.name], 'function', `unknown imported function ${imp.name}`);
  }
  const instance = new WebAssembly.Instance(module, { atto });
  assert.equal(typeof instance.exports.main, 'function');
  const answer = instance.exports.main();
  assert.equal(typeof answer, 'bigint');
  return answer;
}
function pass(code, id, expected) {
  const first = compile(code, `${id}-first`);
  assert.equal(first.result.status, 0, first.result.stderr);
  assert.ok(first.bytes, 'compiler emitted no guest artifact');
  const second = compile(code, `${id}-second`);
  assert.equal(second.result.status, 0, second.result.stderr);
  assert.deepEqual(first.bytes, second.bytes, 'guest output not deterministic');
  assert.equal(instantiate(first.bytes), BigInt(expected), id);
}
function reject(code, id, diagnostic) {
  const out = compile(code, id);
  assert.equal(out.result.status, 1, 'invalid source must exit as compiler rejection');
  assert.equal(out.bytes, null, 'compiler must not emit guest artifact on rejection');
  assert.match(out.result.stderr, /E_[A-Z_]+/, 'expected positioned compiler diagnostic');
  if (diagnostic) assert.ok(out.result.stderr.includes(diagnostic), out.result.stderr);
}
try {
  pass(header + `pub fnc main(): int {
    val a = Particle{x: 10, y: 20};
    val b = Particle(x=10, y=20);
    return a.x + b.y;
  }`, 'equal-forms', 30);
  pass(header + `pub fnc main(): int {
    val p = Particle(y=20, x=10);
    return p.x + p.y;
  }`, 'reordered-literals', 30);
  reject(header + `pub fnc main(): int {
    val p = Particle(x=10, x=11, y=20);
    return p.x;
  }`, 'duplicate-field', 'E_DUPLICATE');
  reject(header + `pub fnc main(): int {
    val p = Particle(x=10, y=20, extra=3);
    return p.x;
  }`, 'unknown-field', 'E_NO_FIELD');
  reject(header + `pub fnc main(): int {
    val Particle = 7;
    val p = Particle(x=10, y=20);
    return p.x;
  }`, 'shadowed-nominal', 'E_STRUCT_RESOLVE');
  reject(header + `pub fnc main(): int {
    val p = Particle(x=10, y:20);
    return p.x;
  }`, 'mixed-delimiters', 'E_STRUCT_INIT');
  reject(header + `pub fnc main(): int {
    val p = new Particle(x=10, y=20);
    return p.x;
  }`, 'explicit-new', 'E_STRUCT_TARGET');
  reject(header + `pub fnc main(): int {
    val p = Particle(y=read_y(), x=10);
    return p.x;
  }`, 'effectful-value-gated', 'E_UNSUPPORTED');
  reject(`fnc add(int value) -> int { return value+1; }
    pub fnc main(): int { return add(value=41); }`,
  'ordinary-equals-call');
  console.log('STRUCT_EQUALS_COMPILED_GUEST_PASS scalar=2 negatives=7');
} finally {
  rmSync(dir, { recursive: true, force: true });
}

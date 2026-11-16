// Real compiled Atto guest execution, not a source-only regex check.
// Uses native Rust attoc to emit real guest Wasm and Node Wasm engine to
// execute imported typed stdio.print operations.
import assert from 'node:assert/strict';
import {spawnSync} from 'node:child_process';
import {mkdtempSync, writeFileSync, rmSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {join} from 'node:path';

const compiler = process.env.ATTOC || 'target/release/attoc';
const dir = mkdtempSync(join(tmpdir(), 'atto-io-'));
const decoder = new TextDecoder('utf-8', {fatal: true});

function compile(source) {
  const file = join(dir, 'case.atto');
  writeFileSync(file, source, 'utf8');
  const r = spawnSync(compiler, [file], {maxBuffer: 2 << 20});
  return {status: r.status, wasm: r.stdout, diagnostic: r.stderr.toString()};
}
function runGuest(wasm) {
  assert.ok(WebAssembly.validate(wasm), 'compiled output is valid real Wasm');
  const mod = new WebAssembly.Module(wasm);
  const imports = WebAssembly.Module.imports(mod).map(i => i.module + '.' + i.name);
  assert.deepEqual(imports.sort(), ['atto.print_i64', 'atto.print_str'], 'output-only ABI imports');
  let output = '';
  let instance;
  instance = new WebAssembly.Instance(mod, {atto: {
    print_i64(value) { output += String(value); },
    print_str(ptr, len) {
      output += decoder.decode(new Uint8Array(instance.exports.memory.buffer, ptr, len));
    }
  }});
  assert.equal(instance.exports.main(), 7n);
  return output;
}
try {
  const shorthand = [
    'pub fnc main(): int {',
    '  val x: int = 42;',
    '  stdio.out << "effectful output";',
    '  writech stdio.out, "!";',
    '  stdio.out << x;',
    '  return 7;',
    '}'
  ].join('\n');
  const explicit = [
    'pub fnc main(): int {',
    '  val x: int = 42;',
    '  stdio.print("effectful output");',
    '  stdio.print("!");',
    '  stdio.print(x);',
    '  return 7;',
    '}'
  ].join('\n');
  const a = compile(shorthand);
  const b = compile(explicit);
  assert.equal(a.status, 0, a.diagnostic);
  assert.equal(b.status, 0, b.diagnostic);
  assert.deepEqual(a.wasm, b.wasm, 'same typed Wasm for shorthand and explicit operations');
  assert.equal(runGuest(a.wasm), 'effectful output!42');
  console.log('PASS compiled Atto output shorthand / explicit I/O Wasm guest');

  for (const [label, src, expected] of [
    ['read not implemented', 'pub fnc main(): void { stdio.in >> val line; }', 'E_UNSUPPORTED'],
    ['other stream unimplemented', 'pub fnc main(): void { output_stream << "x"; }', 'E_UNSUPPORTED'],
    ['select unimplemented', 'pub fnc main(): void { do select { when inbox >> val item -> {} } }', 'E_UNSUPPORTED'],
    ['runtime const read rejected', 'pub fnc main(): void { stdio.in >> const line; }', 'E_CONST_EFFECT'],
    ['read-only stream write rejected', 'pub fnc main(): void { stdio.in << "x"; }', 'E_UNSUPPORTED'],
  ]) {
    const r = compile(src);
    assert.equal(r.status, 1, label);
    assert.equal(r.wasm.length, 0, label+' emitted an artifact');
    assert.ok(r.diagnostic.includes(expected), label+': '+r.diagnostic);
  }
  console.log('PASS unsupported I/O and select fail-closed; runtime const read rejected');
} finally {
  rmSync(dir, {recursive: true, force: true});
}

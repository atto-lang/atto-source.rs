// Execute the actual CLI compiler's Wasm and ESM outputs from identical source.
import assert from 'node:assert/strict';
import { execFileSync, spawnSync } from 'node:child_process';
import { existsSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { pathToFileURL } from 'node:url';

const compiler = resolve(process.env.ATTO_COMPILER_BIN ?? 'target/debug/attoc');
const dir = mkdtempSync(join(tmpdir(), 'atto-ctor-parity-'));
const program = (fields, body, value) => `define class Pair as
${fields}
  constructor(int x, int y) {
${body}
  }
end
pub fnc main(): int {
  let Pair p = new Pair(19, 23);
  return ${value};
}
`;
const one = '  pub final int a;';
const two = one + '\n  pub final int b;';
const cases = [
  ['positive', program(two, '    self.b = y;\n    self.a = x;\n    return;', 'p.a + p.b'), null],
  ['duplicate', program(one, '    self.a = x;\n    self.a = y;', 'p.a'), /E_INIT_ONCE/],
  ['unknown', program(one, '    self.a = z;', 'p.a'), /E_UNBOUND/],
  ['missing', program(two, '    self.a = x;', 'p.a'), /E_INIT:/],
  // This is currently an unsupported runtime operation in the browser frontend;
  // its rejection is not evidence of full typed constructor effect checking.
  ['effect', program(one, '    self.a = x;\n    rt cooperate();', 'p.a'), /E_UNSUPPORTED/],
];
try {
  for (const [name, source, expectedError] of cases) {
    const input = join(dir, name + '.atto');
    writeFileSync(input, source);
    const diagnostics = [];
    for (const [mode, args, extension] of [['wasm', [], 'wasm'], ['js', ['--emit=js'], 'mjs']]) {
      const out = join(dir, name + '.' + extension);
      if (expectedError) {
        const result = spawnSync(compiler, [...args, input, '-o', out], { encoding: 'utf8' });
        assert.ifError(result.error);
        assert.equal(result.signal, null);
        assert.notEqual(result.status, 0, name + ' ' + mode);
        assert.equal(existsSync(out), false, name + ' ' + mode);
        assert.match(result.stderr, expectedError, name + ' ' + mode);
        diagnostics.push(result.stderr);
        continue;
      }
      execFileSync(compiler, [...args, input, '-o', out], { stdio: 'pipe' });
      const guest = mode === 'wasm'
        ? new WebAssembly.Instance(new WebAssembly.Module(readFileSync(out)), {}).exports
        : (await import(pathToFileURL(out).href)).instantiate().exports;
      assert.equal(guest.main(), 42n, mode);
      assert.equal(guest.panic_code.value, 0, mode);
      assert.equal(guest.main(), 42n, mode + ' repeat');
      assert.equal(guest.panic_code.value, 0, mode + ' repeat');
      console.log('CONSTRUCTOR_GUEST_EXECUTED', mode);
    }
    if (expectedError) {
      assert.equal(diagnostics[0], diagnostics[1], name + ' diagnostic parity');
      console.log('CONSTRUCTOR_REJECTED_BOTH', name);
    }
  }
} finally {
  rmSync(dir, { recursive: true, force: true });
}

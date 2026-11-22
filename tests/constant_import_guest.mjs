// Exact-compiler executable Wasm guest tests for the bounded relative-file
// literal constant import subset. Requires a built attoc; no mocked compiler.
import assert from 'node:assert/strict';
import { mkdtempSync, writeFileSync, readFileSync, rmSync, existsSync, symlinkSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { spawnSync } from 'node:child_process';

const compiler = resolve(process.env.ATTOC || 'target/release/attoc');
const dir = mkdtempSync(join(tmpdir(), 'atto-const-import-'));

function compile(main, provider) {
  writeFileSync(join(dir, 'main.atto'), main);
  writeFileSync(join(dir, 'config.atto'), provider);
  const output = join(dir, 'compiled.wasm');
  // Ensure a failed build cannot be mistaken for a prior successful artifact.
  rmSync(output, { force: true });
  const result = spawnSync(compiler, ['main.atto', '-o', output], {
    cwd: dir, encoding: 'utf8', timeout: 30000,
    env: { ...process.env, status_valid: 'unexpected-build-env' },
  });
  return { result, output };
}

function guest(main, provider, expected) {
  const { result, output } = compile(main, provider);
  assert.equal(result.status, 0, result.stderr);
  assert.ok(existsSync(output), 'compiler must create actual guest artifact');
  const wasm = readFileSync(output);
  assert.ok(WebAssembly.validate(wasm), 'emitted binary is valid WebAssembly');
  const module = new WebAssembly.Module(wasm);
  assert.deepEqual(WebAssembly.Module.imports(module), [], 'literal imports need no host access');
  const instance = new WebAssembly.Instance(module, {});
  assert.equal(instance.exports.main(), BigInt(expected));
  assert.equal(instance.exports.panic_code.value, 0);
  return wasm;
}

function rejected(main, provider, expectedCode) {
  const { result, output } = compile(main, provider);
  assert.equal(result.status, 1, result.stderr);
  assert.match(result.stderr, new RegExp(expectedCode));
  assert.equal(existsSync(output), false, 'failed compile must produce no guest artifact');
}

try {
  const main = 'import const status_valid from "./config.atto"; pub fnc main(): int {return status_valid;}';
  const provider = 'define pub const int status_valid = 200;';
  const first = guest(main, provider, 200);
  const second = guest(main, provider, 200);
  assert.deepEqual(first, second, 'identical source must emit byte-identical Wasm');
  guest(main,
    'use namespace std.http; pub fnc unrelated(): int { return 3; } ' + provider,
    200);
  guest('import const status_valid as ok from "./config.atto"; pub fnc main(): int {return ok;}',
    provider, 200);
  guest('import const flag from "./config.atto"; pub fnc main(): int {if flag then return 1; else return 0; fi}',
    'define pub const bool flag = true;', 1);
  guest('import const state from "./config.atto"; pub fnc main(): int {if state eq "ready" then return 42; else return 0; fi}',
    'define pub const string state = "ready";', 42);

  rejected(main.replace('status_valid;', 'unknown;'), provider, 'E_UNBOUND');
  rejected('import const status_valid from "./config.atto"; pub fnc main(): int {status_valid = 5; return 0;}',
    provider, 'E_ASSIGN_CONST');
  rejected(main, 'define const int status_valid = 200;', 'E_PRIVATE');
  rejected(main, 'pub fnc status_valid(): int { return 200; }', 'E_IMPORT_KIND');
  rejected(main, 'define pub const int status_valid = "bad";', 'E_CONST_TYPE');
  rejected(main, 'define pub const string status_valid = std.env.read_env("status_valid");',
    'E_UNSUPPORTED');
  rejected(main, 'x'.repeat(65537), 'E_IMPORT'); // native loader rejects oversized source
  rejected('import const status_valid from "std.http"; pub fnc main(): int {return 0;}',
    provider, 'E_IMPORT_PATH');

  // The real CLI must canonicalize requested paths and reject an imported
  // symlink to source outside the source project's canonical directory.
  const external = mkdtempSync(join(tmpdir(), 'atto-const-import-outside-'));
  try {
    writeFileSync(join(external, 'secret.atto'), provider);
    symlinkSync(join(external, 'secret.atto'), join(dir, 'escape.atto'));
    rejected('import const status_valid from "./escape.atto"; pub fnc main(): int {return 0;}',
      provider, 'E_IMPORT');
  } finally {
    rmSync(join(dir, 'escape.atto'), { force: true });
    rmSync(external, { recursive: true, force: true });
  }

  console.log('PASS: real compiled scalar const imports, deterministic Wasm guests and fail-closed cases');
} finally {
  rmSync(dir, { recursive: true, force: true });
}

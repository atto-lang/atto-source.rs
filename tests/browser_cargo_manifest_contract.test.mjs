import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';

const manifest = readFileSync(new URL('../Cargo.toml', import.meta.url), 'utf8');

function inventory(text) {
  const bins = [...text.matchAll(
    /\[\[bin\]\]\s*name\s*=\s*"([^"]+)"\s*path\s*=\s*"([^"]+)"/g
  )].map((m) => [m[1], m[2]]);
  const tests = [...text.matchAll(
    /\[\[test\]\]\s*name\s*=\s*"([^"]+)"\s*path\s*=\s*"tests\/([^"]+)\.rs"/g
  )].map((m) => [m[1], m[2]]);
  assert.equal(bins.length, (text.match(/^\[\[bin\]\]\s*$/gm) ?? []).length,
    'every bin must declare an explicit name and path');
  assert.equal(tests.length, (text.match(/^\[\[test\]\]\s*$/gm) ?? []).length,
    'every test must declare an explicit name and path');
  assert.equal(new Set(bins.map(([name]) => name)).size, bins.length,
    'duplicate binary name');
  assert.equal(new Set(bins.map(([, path]) => path)).size, bins.length,
    'duplicate binary path');
  assert.equal(new Set(tests.map(([name]) => name)).size, tests.length,
    'duplicate test name');
  assert.equal(new Set(tests.map(([, path]) => path)).size, tests.length,
    'duplicate test path');
  for (const [name, path] of tests) assert.equal(name, path,
    'integration target path/name drift');
  return { bins, tests };
}

test('browser compiler has exactly two explicit binaries, not an implicit native CLI', () => {
  assert.match(manifest, /^autobins\s*=\s*false\s*$/m);
  assert.match(manifest, /^autotests\s*=\s*false\s*$/m);
  assert.deepEqual(inventory(manifest).bins, [
    ['attoc', 'src/bin/attoc.rs'],
    ['atto-state-mir', 'src/bin/atto_state_mir.rs']
  ]);
});

test('explicit Cargo test inventory includes every expected browser target', () => {
  const { tests } = inventory(manifest);
  assert.equal(tests.length, 92,
    'manifest target inventory changed; update the full on-disk audit and private mirror');
  assert.ok(tests.some(([name]) => name === 'projected_actor_state_receivers'));
  assert.ok(tests.some(([name]) => name === 'actor_state_mir_v0'));
  assert.ok(tests.some(([name]) => name === 'projected_receiver_cli_denial'));
  assert.ok(tests.every(([name]) => name !== 'actor_arena_contract'),
    'native Cranelift suites must not be falsely counted as browser tests');
});

test('duplicate, missing and injected Cargo targets fail closed', () => {
  const original = inventory(manifest);
  assert.throws(() => inventory(manifest.replace(
    'path = "src/bin/attoc.rs"', 'path = "invalid"\n# missing attoc target'
  )), /every bin must declare/);
  assert.throws(() => inventory(manifest + '\n[[bin]]\nname = "attoc"\npath = "src/bin/other.rs"\n'),
    /duplicate binary name/);
  assert.throws(() => inventory(manifest + '\n[[test]]\nname = "actor_state_mir_v0"\npath = "tests/other.rs"\n'),
    /duplicate test name/);
  assert.throws(() => inventory(manifest.replace(
    'path = "tests/actor_state_mir_v0.rs"',
    'path = "tests/actor_state_mir_v0.rs"\n[[test]]\nname = "hidden"'
  )), /every test must declare/);
  assert.equal(original.tests.length, 92);
});

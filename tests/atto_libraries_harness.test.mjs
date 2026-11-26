// Host-only harness controls. These never certify an Atto compiler or guest.
import test from 'node:test';
import assert from 'node:assert/strict';
import { copyFileSync, mkdirSync, mkdtempSync, writeFileSync, rmSync } from 'node:fs';
import { join } from 'node:path';
import { tmpdir } from 'node:os';
import { spawnSync } from 'node:child_process';

const names = ["core_num","core_cmp","core_option_result","core_mem","alloc_arena","std_io","std_path_posix","std_path_windows","aliases","neg_unknown_core","neg_unknown_std","neg_traversal","neg_nested","neg_private","neg_dotted_selector","neg_ores_selector"];
test('missing baseline corpus cannot be reported as a pass', () => {
  const root = mkdtempSync(join(tmpdir(), 'atto-library-runner-test-'));
  try {
    const script = join(root, 'runner.mjs');
    copyFileSync(new URL('./atto_libraries.mjs', import.meta.url), script);
    const corpus = join(root, 'atto_libraries');
    mkdirSync(corpus);
    const run = () => spawnSync(process.execPath, [script, join(root, 'no-compiler')], { encoding: 'utf8', timeout: 10000 });
    for (const name of names.slice(0, -1)) {
      writeFileSync(join(corpus, name + '.atto'), '// expect-reject\n');
    }
    const partial = run();
    assert.equal(partial.status, 1);
    assert.match(partial.stderr, /missing required library cases: neg_ores_selector.atto/);
    assert.doesNotMatch(partial.stdout, /passed/);
    writeFileSync(join(corpus, 'neg_ores_selector.atto'), '// expect-reject\n');
    const absent = run();
    assert.equal(absent.status, 1);
    assert.match(absent.stdout, /unexpected fixture polarity/);
    assert.match(absent.stdout, /compiler process did not complete normally/);
    assert.match(absent.stdout, /0\/16 passed/);
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});

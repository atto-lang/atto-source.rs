// Provenance guards ONLY. These checks never build Atto or execute guest Wasm.
import assert from 'node:assert/strict';
import { test } from 'node:test';
import { spawnSync, execFileSync } from 'node:child_process';
import { copyFileSync, mkdtempSync, mkdirSync, rmSync, symlinkSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const script = resolve(dirname(fileURLToPath(import.meta.url)), '../scripts/verify-list-pattern-guest.sh');
function git(cwd, ...args) {
  return execFileSync('git', args, { cwd, encoding: 'utf8' }).trim();
}
function which(command) {
  return execFileSync('/bin/sh', ['-c', 'command -v ' + command], { encoding: 'utf8' }).trim();
}

test('exact-head and whole-tree mirror provenance fail closed', () => {
  const root = mkdtempSync(join(tmpdir(), 'atto-list-provenance-'));
  const checkout = join(root, 'checkout');
  const bin = join(root, 'bin');
  try {
    mkdirSync(join(checkout, 'scripts'), { recursive: true });
    mkdirSync(bin);
    copyFileSync(script, join(checkout, 'scripts/verify-list-pattern-guest.sh'));
    symlinkSync(which('git'), join(bin, 'git'));
    symlinkSync(which('dirname'), join(bin, 'dirname'));
    git(checkout, 'init', '-q');
    git(checkout, 'config', 'user.email', 'proof-test@example.invalid');
    git(checkout, 'config', 'user.name', 'Proof guard test');
    git(checkout, 'add', 'scripts/verify-list-pattern-guest.sh');
    git(checkout, 'commit', '-qm', 'pinned source');
    const originalHead = git(checkout, 'rev-parse', 'HEAD');
    const originalTree = git(checkout, 'rev-parse', 'HEAD^{tree}');
    const one = (overrides = {}) => {
      const env = {
        ...process.env,
        PATH: bin, // Deliberately no cargo, rustc, or node: no guest can execute.
        ATTO_EXPECT_HEAD: originalHead,
        ...overrides,
      };
      const p = spawnSync('/bin/bash', [join(checkout, 'scripts/verify-list-pattern-guest.sh')], {
        cwd: checkout, env, encoding: 'utf8',
      });
      assert.equal(p.error, undefined);
      return p;
    };
    const check = (overrides, status, diagnostic) => {
      const p = one(overrides);
      assert.equal(p.status, status, p.stderr + p.stdout);
      assert.match(p.stderr, diagnostic);
    };

    check({ ATTO_EXPECT_HEAD: 'bad' }, 2, /ATTO_EXPECT_HEAD/);
    check({ ATTO_EXPECT_HEAD: '0'.repeat(40) }, 2, /SOURCE_SHA_MISMATCH/);
    check({}, 3, /MISSING_CARGO/);
    check({ ATTO_EXPECT_TREE: '0'.repeat(40) }, 2, /SOURCE_TREE_MISMATCH/);
    check({ ATTO_VERIFY_MODE: 'accept-any' }, 2, /INVALID_VERIFY_MODE/);

    // A carrier commit is NOT the source commit even if its entire Git tree matches.
    git(checkout, 'commit', '--amend', '-qm', 'private carrier');
    assert.notEqual(git(checkout, 'rev-parse', 'HEAD'), originalHead);
    assert.equal(git(checkout, 'rev-parse', 'HEAD^{tree}'), originalTree);
    check({ ATTO_VERIFY_MODE: 'mirror' }, 2, /MIRROR_REQUIRES_PINNED_SOURCE_TREE/);
    check({ ATTO_VERIFY_MODE: 'mirror', ATTO_EXPECT_TREE: 'not-a-sha' }, 2, /INVALID_SOURCE_TREE_SHA/);
    check({ ATTO_VERIFY_MODE: 'mirror', ATTO_EXPECT_TREE: '0'.repeat(40) }, 2, /SOURCE_TREE_MISMATCH/);
    check({ ATTO_VERIFY_MODE: 'mirror', ATTO_EXPECT_TREE: originalTree }, 3, /MISSING_CARGO/);

    writeFileSync(join(checkout, 'untracked.txt'), 'dirty');
    check({ ATTO_VERIFY_MODE: 'mirror', ATTO_EXPECT_TREE: originalTree }, 2, /SOURCE_TREE_DIRTY/);
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});

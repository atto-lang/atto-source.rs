// Executes native-compiled guests against the logical call-depth quota
// (language spec chapter 25; chapter 18 "Stacks" rules 1, 5 and 7).
// A call on a recursive cycle of the call graph may nest at most
// MAX_CALL_DEPTH = 1024 deep; the next one sets panic_code 6 and traps.
// The bound is the same in every engine, unlike the engine's own RangeError.
import assert from 'node:assert/strict';
import { mkdtemp, writeFile, readFile, rm } from 'node:fs/promises';
import { spawnSync } from 'node:child_process';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';

const attoc = process.env.ATTOC || fileURLToPath(new URL('../target/debug/attoc', import.meta.url));
const PANIC_STACK = 6;
const QUOTA = 1024;
// i32.const 1024 (sleb 0x80 0x08); i32.ge_u: the quota check prelude.
const PRELUDE = [0x41, 0x80, 0x08, 0x4f];

const recurse = depth => `fnc sum(int n): int { if n == 0 then return 0; fi return sum(n - 1) + 1; }
pub fnc main(): int { return sum(${depth}); }`;

const cases = [
  { name: 'exactly the quota of recursive frames completes', src: recurse(QUOTA), value: BigInt(QUOTA) },
  { name: 'one recursive frame past the quota panics with code 6', src: recurse(QUOTA + 1), panic: PANIC_STACK },
  {
    name: 'mutual recursion shares one quota',
    src: `fnc even(int n): bool { if n == 0 then return true; fi return odd(n - 1); }
    fnc odd(int n): bool { if n == 0 then return false; fi return even(n - 1); }
    pub fnc main(): int { if even(${QUOTA}) then return 1; fi return 0; }`,
    value: 1n,
  },
  {
    name: 'mutual recursion past the quota panics',
    src: `fnc even(int n): bool { if n == 0 then return true; fi return odd(n - 1); }
    fnc odd(int n): bool { if n == 0 then return false; fi return even(n - 1); }
    pub fnc main(): int { if even(${QUOTA + 1}) then return 1; fi return 0; }`,
    panic: PANIC_STACK,
  },
  {
    // Without restoring the depth at the catch landing pad, each caught
    // throw would leak 900 frames and the second round would panic.
    name: 'a caught throw from deep recursion restores the call depth',
    src: `fnc boom(int n): int { if n == 0 then throw Error("x"); fi return boom(n - 1) + 1; }
    fnc guarded(): int { try { return boom(900); } catch (Error e) { return 1; } }
    pub fnc main(): int {
      let int total = 0;
      for (let i = 0; i < 50; i++) do
        total += guarded();
      done
      return total;
    }`,
    value: 50n,
  },
  {
    name: 'a finally on the recursive path restores the depth before rethrow',
    src: `fnc boom(int n): int {
      if n == 0 then throw Error("x"); fi
      try { return boom(n - 1) + 1; } finally { }
    }
    fnc guarded(): int { try { return boom(600); } catch (Error e) { return 1; } }
    pub fnc main(): int {
      let int total = 0;
      for (let i = 0; i < 20; i++) do
        total += guarded();
      done
      return total;
    }`,
    value: 20n,
  },
  {
    name: 'non-recursive calls are statically bounded and never instrumented',
    src: `fnc a(int n): int { return n + 1; }
    fnc b(int n): int { return a(n) + a(n); }
    pub fnc main(): int { return b(20); }`,
    value: 42n,
    noPrelude: true,
  },
];

function runGuest(bytes) {
  assert.ok(WebAssembly.validate(bytes), 'invalid guest');
  const instance = new WebAssembly.Instance(new WebAssembly.Module(bytes), { atto: {
    print_i64() {}, print_bool() {}, print_f64() {}, print_str() {}, print_newline() {},
  } });
  try {
    return { value: instance.exports.main(), panic: instance.exports.panic_code.value };
  } catch (error) {
    return { error, panic: instance.exports.panic_code.value };
  }
}

function contains(bytes, pattern) {
  outer: for (let i = 0; i + pattern.length <= bytes.length; i++) {
    for (let j = 0; j < pattern.length; j++) if (bytes[i + j] !== pattern[j]) continue outer;
    return true;
  }
  return false;
}

const work = await mkdtemp(join(tmpdir(), 'atto-call-depth-'));
try {
  for (const [i, c] of cases.entries()) {
    const src = join(work, `case${i}.atto`);
    const out = join(work, `case${i}.wasm`);
    await writeFile(src, c.src);
    const built = spawnSync(attoc, [src, '-o', out], { encoding: 'utf8', timeout: 30_000 });
    assert.equal(built.status, 0, c.name + ': attoc failed: ' + built.stderr);
    const bytes = await readFile(out);
    if (c.noPrelude) assert.ok(!contains(bytes, PRELUDE), c.name + ': unexpected quota prelude');
    else assert.ok(contains(bytes, PRELUDE), c.name + ': recursive call lacks the quota prelude');
    const r = runGuest(bytes);
    if (c.panic) {
      assert.ok(r.error instanceof WebAssembly.RuntimeError,
        c.name + ': expected a deterministic trap, got ' + r.error);
      assert.ok(!(r.error instanceof RangeError), c.name + ': engine stack exhausted before the quota');
      assert.equal(r.panic, c.panic, c.name + ': panic code');
    } else {
      assert.ok(!r.error, c.name + ': unexpected failure ' + r.error);
      assert.equal(r.value, c.value, c.name + ': return value');
      assert.equal(r.panic, 0, c.name + ': unexpected panic');
    }
    console.log('PASS call-depth guest:', c.name);
  }
} finally {
  await rm(work, { recursive: true, force: true });
}

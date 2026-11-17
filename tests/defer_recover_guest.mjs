// Executes native-compiled guests for the defer/recover/try/raise slice and
// checks exact observable traces against atto-spec
// language/defer-recover-unwind-v1.md "Mandatory conformance traces".
// Oracles are written from the specification, not captured from the compiler.
import assert from 'node:assert/strict';
import { mkdtemp, writeFile, readFile, rm } from 'node:fs/promises';
import { spawnSync } from 'node:child_process';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';

const attoc = process.env.ATTOC || fileURLToPath(new URL('../target/debug/attoc', import.meta.url));

const cases = [
  {
    name: 'normal return runs two defers LIFO after snapshotting the value',
    src: `fnc f(): int {
      let int x = 4;
      defer || -> { stdio.println("A"); };
      defer || -> { stdio.println("B"); };
      stdio.println("body");
      return x;
    }
    pub fnc main(): int { return f(); }`,
    stdout: 'body\nB\nA\n', value: 4n,
  },
  {
    name: 'a defer registered after an early return never runs',
    src: `fnc f(bool early): int {
      defer || -> { stdio.println("first"); };
      if early then return 1; fi
      defer || -> { stdio.println("second"); };
      return 2;
    }
    pub fnc main(): int { return f(true) * 10 + f(false); }`,
    stdout: 'first\nsecond\nfirst\n', value: 12n,
  },
  {
    name: 'nested finally blocks run inner to outer on return; value is a snapshot',
    src: `fnc f(): int {
      let int x = 1;
      try {
        try { return x; } finally { stdio.println("inner"); x = 50; }
      } finally { stdio.println("outer"); }
    }
    pub fnc main(): int { return f(); }`,
    stdout: 'inner\nouter\n', value: 1n,
  },
  {
    name: 'finally inside a loop runs once per iteration',
    src: `pub fnc main(): int {
      let int total = 0;
      for (let i = 0; i < 3; i++) do
        try { total += i; } finally { stdio.println(i); }
      done
      return total;
    }`,
    stdout: '0\n1\n2\n', value: 3n,
  },
  {
    name: 'raise inside try/catch: finally, cleanup, recover; never catch',
    src: `fnc fail(): int raises(Error) { raise new Error("r"); }
    fnc f(): int {
      recover (|int v| -> |err, c| -> { stdio.println("recover"); return v; })(9);
      defer || -> { stdio.println("cleanup"); };
      try { return fail(); } catch (Error e) { return -1; } finally { stdio.println("finally"); }
    }
    pub fnc main(): int { return f(); }`,
    stdout: 'finally\ncleanup\nrecover\n', value: 9n,
  },
  {
    name: 'throw handled locally: catch, finally, cleanup; never recover',
    src: `fnc t(): int throws(Error) { throw new Error("t"); }
    fnc f(): int {
      recover (|int v| -> |err, c| -> { stdio.println("UNEXPECTED"); return v; })(9);
      defer || -> { stdio.println("cleanup"); };
      try { return t(); } catch (Error e) { stdio.println("catch"); return 21; } finally { stdio.println("finally"); }
    }
    pub fnc main(): int { return f(); }`,
    stdout: 'catch\nfinally\ncleanup\n', value: 21n,
  },
  {
    name: 'throw from a callee with recover is handled by the outer caller catch',
    src: `fnc callee(): int throws(Error) {
      recover (|int v| -> |err, c| -> { stdio.println("UNEXPECTED"); return v; })(-10);
      defer || -> { stdio.println("callee defer"); };
      throw Error("x");
    }
    pub fnc main(): int {
      try { return callee(); } catch (Error e) { stdio.println("caller catch"); return 21; }
      finally { stdio.println("caller finally"); }
    }`,
    stdout: 'callee defer\ncaller catch\ncaller finally\n', value: 21n,
  },
  {
    name: 'legacy caller-side Option trap is rejected pending Result lowering',
    src: `fnc callee(): int {
      recover (|int v| -> |err, c| -> { stdio.println("UNEXPECTED"); return v; })(-1);
      throw Error("x");
    }
    pub fnc main(): int {
      val Option<int> r = trap callee();
      match r over
        on Some(v) -> { stdio.println(v); }
        on None -> { stdio.println("trap won"); }
      end
      return 3;
    }`,
    reject: 'E_UNSUPPORTED', // no guest: new Result trap must be implemented first
  },
  {
    name: 'throw through nested calls: inner defers, outer defers, then recover',
    src: `fnc inner(): int { defer || -> { stdio.println("inner"); }; throw Error("x"); }
    fnc mid(): int { defer || -> { stdio.println("mid"); }; return inner(); }
    fnc top(): int {
      recover (|int v| -> |err, c| -> { stdio.println("recover"); return v; })(11);
      return mid();
    }
    pub fnc main(): int { return top() + top(); }`,
    stdout: 'inner\nmid\nrecover\ninner\nmid\nrecover\n', value: 22n,
  },
  {
    name: 'recovery delegates: newest c.next(), older recover sees it once',
    src: `fnc fail(): int raises(Error) { raise new Error("r"); }
    fnc f(): int {
      recover (|int v| -> |err, c| -> { stdio.println("older"); return v; })(5);
      recover (|string s| -> |err, c| -> { stdio.println(s); c.next(); })("newest");
      return fail();
    }
    pub fnc main(): int { return f(); }`,
    stdout: 'newest\nolder\n', value: 5n,
  },
  {
    name: 'all callbacks decline: original failure escapes after cleanup',
    src: `fnc f(): int {
      recover (|string s| -> |err, c| -> { stdio.println(s); c.next(); })("declined");
      defer || -> { stdio.println("cleanup"); };
      raise Error("escapes");
    }
    pub fnc main(): int { return f(); }`,
    stdout: 'cleanup\ndeclined\n', escapes: true,
  },
  {
    name: 'raise is never matched by catch',
    src: `pub fnc main(): int {
      try { raise Error("escalated"); } catch (Error e) { return 1; }
    }`,
    stdout: '', escapes: true,
  },
  {
    name: 'panic bypasses catch, finally, defer and recover',
    src: `fnc f(int big): int {
      recover (|int v| -> |err, c| -> { return v; })(7);
      defer || -> { stdio.println("UNEXPECTED defer"); };
      try { return big * big; } catch (Error e) { return -1; }
      finally { stdio.println("UNEXPECTED finally"); }
    }
    pub fnc main(): int { return f(4611686018427387904); }`,
    stdout: '', panic: 1,
  },
];

function runGuest(bytes) {
  assert.ok(WebAssembly.validate(bytes), 'invalid guest');
  const module = new WebAssembly.Module(bytes);
  for (const imp of WebAssembly.Module.imports(module)) {
    assert.equal(imp.module, 'atto');
    assert.ok(['print_i64', 'print_bool', 'print_str', 'print_newline', 'print_f64'].includes(imp.name),
      'unexpected host import ' + imp.name);
  }
  let memory;
  let stdout = '';
  const dec = new TextDecoder();
  const instance = new WebAssembly.Instance(module, { atto: {
    print_i64: v => { stdout += String(v); },
    print_bool: v => { stdout += v ? 'true' : 'false'; },
    print_f64: v => { stdout += String(v); },
    print_str: (p, n) => { stdout += dec.decode(new Uint8Array(memory.buffer, p, n)); },
    print_newline: () => { stdout += '\n'; },
  } });
  memory = instance.exports.memory;
  try {
    return { value: instance.exports.main(), stdout, panic: instance.exports.panic_code.value };
  } catch (error) {
    return { error, stdout, panic: instance.exports.panic_code.value };
  }
}

const work = await mkdtemp(join(tmpdir(), 'atto-unwind-'));
try {
  for (const [i, c] of cases.entries()) {
    const src = join(work, `case${i}.atto`);
    const out = join(work, `case${i}.wasm`);
    await writeFile(src, c.src);
    const built = spawnSync(attoc, [src, '-o', out], { encoding: 'utf8', timeout: 30_000 });
    if (c.reject) {
      assert.notEqual(built.status, 0, c.name + ': legacy Option trap must not compile');
      assert.match(built.stderr, new RegExp(c.reject), c.name + ': expected typed diagnostic');
      await assert.rejects(readFile(out), { code: 'ENOENT' }, 'failure must leave no guest artifact');
      console.log('PASS negative no-artifact:', c.name);
      continue;
    }
    assert.equal(built.status, 0, c.name + ': attoc failed: ' + built.stderr);
    const r = runGuest(await readFile(out));
    assert.equal(r.stdout, c.stdout, c.name + ': stdout');
    if (c.escapes) {
      // A Wasm exception carrying a private tag: never an engine trap or panic.
      assert.ok(r.error instanceof WebAssembly.Exception, c.name + ': expected escaping guest failure, got ' + r.error);
      assert.equal(r.panic, 0, c.name + ': failure must not be relabelled a panic');
    } else if (c.panic) {
      assert.ok(r.error instanceof WebAssembly.RuntimeError, c.name + ': expected engine trap');
      assert.equal(r.panic, c.panic, c.name + ': panic code');
    } else {
      assert.ok(!r.error, c.name + ': unexpected failure ' + r.error);
      assert.equal(r.value, c.value, c.name + ': return value');
      assert.equal(r.panic, 0, c.name + ': unexpected panic');
    }
    console.log('PASS unwind guest:', c.name);
  }
} finally {
  await rm(work, { recursive: true, force: true });
}

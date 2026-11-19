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
    name: 'annotated caught throw is safe in defer and preserves normal result',
    src: `fnc fail(): int throws(Error) { throw Error("inner"); }
      fnc safe(): int raises(Error) {
        try { return fail(); }
        catch (Error e) { stdio.println("handled"); return 9; }
      }
      fnc f(): int {
        defer { stdio.println(safe()); };
        return 35;
      }
      pub fnc main(): int { return f(); }`,
    stdout: 'handled\n9\n', value: 35n,
  },
  {
    name: 'dead throw after terminal return cannot poison cleanup effect',
    src: `fnc safe(): int raises(Error) {
        return 8;
        throw Error("unreachable");
      }
      fnc f(): int {
        defer { stdio.println(safe()); };
        return 36;
      }
      pub fnc main(): int { return f(); }`,
    stdout: '8\n', value: 36n,
  },
  {
    name: 'catch body with no possible ordinary throw is unreachable',
    src: `fnc safe(): int raises(Error) {
        try { return 12; }
        catch (Error e) { throw Error("never"); }
      }
      fnc f(): int {
        defer { stdio.println(safe()); };
        return 37;
      }
      pub fnc main(): int { return f(); }`,
    stdout: '12\n', value: 37n,
  },
  {
    name: 'defer accepts a callee whose ordinary throw is caught locally',
    src: `fnc fail(): int { throw Error("inner"); }
      fnc safe(): int {
        try { return fail(); }
        catch (Error e) { stdio.println("handled"); return 9; }
      }
      fnc f(): int {
        defer { stdio.println(safe()); };
        return 30;
      }
      pub fnc main(): int { return f(); }`,
    stdout: 'handled\n9\n', value: 30n,
  },
  {
    name: 'defer accepts a transitive call whose throw is wrapped in trap',
    src: `fnc fail(): int { throw Error("inner"); }
      fnc safe(): int {
        val Option<int> caught = trap fail();
        stdio.println("trapped");
        return 8;
      }
      fnc helper(): int { return safe() + 1; }
      fnc f(): int {
        defer { stdio.println(helper()); };
        return 31;
      }
      pub fnc main(): int { return f(); }`,
    stdout: 'trapped\n9\n', value: 31n,
  },
  {
    name: 'finally accepts locally caught transitive throw',
    src: `fnc fail(): int { throw Error("inner"); }
      fnc safe(): int {
        try { return fail(); }
        catch (Error e) { return 12; }
      }
      fnc f(): int {
        try { return 32; }
        finally { stdio.println(safe()); }
      }
      pub fnc main(): int { return f(); }`,
    stdout: '12\n', value: 32n,
  },
  {
    name: 'recovery handler may call a function with internally handled throw',
    src: `fnc fail(): int { throw Error("inner"); }
      fnc safe(): int {
        try { return fail(); }
        catch (Error e) { stdio.println("safe catch"); return 17; }
      }
      fnc f(): int {
        recover |err, cursor| -> {
          stdio.println(err.message);
          return safe();
        };
        raise Error("outer original");
      }
      pub fnc main(): int { return f(); }`,
    stdout: 'outer original\nsafe catch\n', value: 17n,
  },
  {
    name: 'an unhandled ordinary throw reaches recovery with is_raise false',
    src: `fnc fail(): int { throw Error("ordinary"); }
      fnc f(): int {
        recover |err, cursor| -> {
          stdio.println(err.message);
          stdio.println(err.is_raise);
          return 44;
        };
        return fail();
      }
      pub fnc main(): int { return f(); }`,
    stdout: 'ordinary\nfalse\n', value: 44n,
  },
  {
    name: 'failed defer factory capture does not register half-initialized cleanup',
    src: `fnc fail_capture(): int { raise Error("capture failed"); }
      fnc f(): int {
        recover |err, cursor| -> {
          stdio.println(err.message);
          return 61;
        };
        defer { stdio.println("prior cleanup"); };
        defer (|int value| -> _ -> { stdio.println("UNEXPECTED cleanup"); })(fail_capture());
        return 99;
      }
      pub fnc main(): int { return f(); }`,
    stdout: 'prior cleanup\ncapture failed\n', value: 61n,
  },
  {
    name: 'safe transitive user calls in defer retain correct return snapshot',
    src: `fnc one(int n): int { return n + 1; }
      fnc two(int n): int { return one(n) + 1; }
      fnc f(): int {
        defer (|int value| -> _ -> { stdio.println(two(value)); })(5);
        return 31;
      }
      pub fnc main(): int { return f(); }`,
    stdout: '7\n', value: 31n,
  },
  {
    name: 'all three ordinary defer spellings execute LIFO exactly once',
    src: `fnc f(): int {
      defer { stdio.println("block"); };
      defer _ -> { stdio.println("underscore"); };
      defer || -> { stdio.println("lambda"); };
      return 12;
    }
    pub fnc main(): int { return f(); }`,
    stdout: 'lambda\nunderscore\nblock\n', value: 12n,
  },
  {
    name: 'defer factory captures a scalar once at registration, before mutation',
    src: `fnc capture(): int { stdio.println("evaluate"); return 7; }
    fnc f(): int {
      let int current = 1;
      defer (|int value| -> _ -> { stdio.println(value); })(capture());
      current = 99;
      stdio.println("body");
      return current;
    }
    pub fnc main(): int { return f(); }`,
    stdout: 'evaluate\nbody\n7\n', value: 99n,
  },
  {
    name: 'canonical recover exposes the original raise message and kind',
    src: `fnc fail(): int raises(Error) { raise Error("the original error"); }
    fnc f(): int {
      recover |ErrorView err, RecoveryCursor cursor| -> {
        stdio.println(err.message);
        stdio.println(err.is_raise);
        return 23;
      };
      defer { stdio.println("cleanup"); };
      return fail();
    }
    pub fnc main(): int { return f(); }`,
    stdout: 'cleanup\nthe original error\ntrue\n', value: 23n,
  },
  {
    name: 'recover delegation sees unchanged error and honors LIFO',
    src: `fnc f(): int {
      recover |original, cursor| -> {
        stdio.println(original.message);
        stdio.println(original.is_raise);
        return 31;
      };
      recover |err, next| -> {
        stdio.println(err.message);
        next.next();
      };
      raise Error("same error");
    }
    pub fnc main(): int { return f(); }`,
    stdout: 'same error\nsame error\ntrue\n', value: 31n,
  },
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
    name: 'a caller-side trap also outranks a callee recover',
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
    stdout: 'trap won\n', value: 3n,
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

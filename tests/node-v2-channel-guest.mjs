// Real Atto .atto -> native attoc -> independently executed Wasm guests.
// A deadlocked rendezvous must fail via a child-process timeout, not stall CI.
// This is NOT an emulated channel scheduler or a stand-in for browser evidence.
import assert from 'node:assert/strict';
import {readFileSync, mkdtempSync, rmSync} from 'node:fs';
import {spawnSync} from 'node:child_process';
import {tmpdir} from 'node:os';
import {join, resolve} from 'node:path';
import {fileURLToPath} from 'node:url';

const cases = [
  ['pass/235-v2-channel-module-fifo.atto', 42, '42\n', 0],
  ['pass/236-v2-unbounded-constructor-atto.atto', 42, '42\n', 0],
  ['pass/238-v2-rendezvous-direct-readch.atto', 42, '42\n', 0],
  ['pass/239-v2-rendezvous-for-await.atto', 42, '42\n', 0],
  ['pass/240-v2-rendezvous-close-wakes-reader.atto', 42, '42\n', 0],
  ['pass/245-v2-try-read-open-empty-and-closed-drain.atto', 42, '42\n', 0],
  ['pass/247-v2-cancel-read-fifo-tail.atto', 42, '42\n', 0],
  ['pass/248-v2-cancel-read-wrong-channel.atto', 42, '42\n', 0],
  ['pass/249-v2-cancel-read-after-delivery.atto', 42, '42\n', 0],
  ['pass/250-v2-cancel-read-after-close.atto', 42, '42\n', 0],
  ['pass/251-v2-cancel-read-head-preserves-fifo.atto', 42, '42\n', 0],
  ['pass/252-v2-cancel-read-middle-preserves-fifo.atto', 42, '42\n', 0],
  ['pass/253-v2-cancel-read-releases-graph-admission.atto', 42, '40\n2\n', 0],
  ['panic/235-v2-negative-bounded-channel.atto', null, '', 14],
  ['panic/236-v2-negative-other-capacity.atto', null, '', 14],
  ['panic/237-v2-negative-legacy-channel-constructor.atto', null, '', 14],
  ['panic/241-v2-legacy-pipe-producer-pending-read-denial.atto', null, '', 10],
  ['panic/242-v2-legacy-pipe-consumer-pending-read-denial.atto', null, '', 10],
  ['panic/243-v2-legacy-pipe-producer-preloaded-denial.atto', null, '', 10],
  ['panic/244-v2-legacy-pipe-consumer-preloaded-denial.atto', null, '', 10],
  ['panic/245-v2-try-read-closed-empty-denial.atto', null, '', 9],
  ['panic/246-v2-try-read-after-drain-denial.atto', null, '42\n', 9],
  ['panic/247-v2-cancel-read-await-traps.atto', null, '', 15],
];

function runSingleGuest(path) {
  const bytes = readFileSync(path);
  assert.ok(WebAssembly.validate(bytes), 'compiler produced invalid Wasm');
  const module = new WebAssembly.Module(bytes);
  const allowed = new Set(['print_i64', 'print_bool', 'print_str', 'print_newline', 'print_f64']);
  for (const imp of WebAssembly.Module.imports(module)) {
    assert.equal(imp.module, 'atto', 'unexpected host import module');
    assert.ok(allowed.has(imp.name), 'unexpected host capability: ' + imp.name);
  }

  let stdout = '', instance;
  const atto = {
    print_i64(v) { stdout += String(v); },
    print_bool(v) { stdout += v ? 'true' : 'false'; },
    print_str(ptr, len) {
      stdout += new TextDecoder('utf-8', {fatal: true}).decode(
        new Uint8Array(instance.exports.memory.buffer, ptr, len));
    },
    print_newline() { stdout += '\n'; },
    print_f64(v) { stdout += String(v); },
  };
  instance = new WebAssembly.Instance(module, {atto});
  let returned = null, trapped = false;
  try {
    returned = String(instance.exports.main());
  } catch (error) {
    if (!(error instanceof WebAssembly.RuntimeError)) throw error;
    trapped = true;
  }
  const panicCode = Number(instance.exports.panic_code.value);
  process.stdout.write(JSON.stringify({stdout, returned, trapped, panicCode}));
}

function executeCorpus() {
  const compiler = resolve(process.env.ATTOC || 'target/debug/attoc');
  const temporary = mkdtempSync(join(tmpdir(), 'atto-channel-real-wasm-'));
  try {
    for (const [suffix, expectedExit, expectedStdout, expectedPanic] of cases) {
      const source = resolve('tests/fixtures/' + suffix);
      const wasmFile = join(temporary, 'channel-guest.wasm');
      const compiled = spawnSync(compiler, [source, '-o', wasmFile], {
        encoding: 'utf8', timeout: 30_000, maxBuffer: 128 * 1024,
      });
      assert.equal(compiled.error, undefined,
        suffix + ': compiler failed to start: ' + compiled.error);
      assert.equal(compiled.status, 0,
        suffix + ': real Atto compiler rejected source: ' + compiled.stderr);

      // Run each Wasm guest in a child process. A scheduler deadlock or
      // infinite busy loop must be killed rather than hanging the CI runner.
      const guest = spawnSync(process.execPath, [
        fileURLToPath(import.meta.url), '--execute-guest', wasmFile,
      ], {
        encoding: 'utf8', timeout: 10_000, maxBuffer: 128 * 1024,
      });
      assert.equal(guest.error, undefined,
        suffix + ': guest timeout/crash: ' + guest.error);
      assert.equal(guest.status, 0,
        suffix + ': Wasm guest execution error: ' + guest.stderr);
      let actual;
      try { actual = JSON.parse(guest.stdout); }
      catch { assert.fail(suffix + ': invalid guest result: ' + guest.stdout); }
      assert.equal(actual.stdout, expectedStdout, suffix + ': stdout');
      assert.equal(actual.panicCode, expectedPanic, suffix + ': guest panic code');
      if (expectedPanic) {
        assert.equal(actual.trapped, true, suffix + ': expected runtime trap');
        assert.equal(actual.returned, null, suffix + ': unexpected return after trap');
      } else {
        assert.equal(actual.trapped, false, suffix + ': unexpectedly trapped');
        assert.equal(actual.returned, String(expectedExit), suffix + ': exit value');
      }
      console.log('REAL_WASM_CHANNEL_' +
        (expectedPanic ? 'DENIAL' : 'PASS') + ' ' + suffix);
    }
  } finally {
    rmSync(temporary, {recursive: true, force: true});
  }
}

if (process.argv[2] === '--execute-guest') {
  assert.ok(process.argv[3], 'one compiler-produced guest Wasm path required');
  runSingleGuest(process.argv[3]);
} else {
  executeCorpus();
}

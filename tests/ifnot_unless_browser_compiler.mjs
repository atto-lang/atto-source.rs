// End-to-end compiler parity for the if/ifnot/unless scalar contract.
// This executes the actual wasm32-hosted Rust compiler, never a JS interpreter.
import assert from 'node:assert/strict';
import {createHash} from 'node:crypto';
import {readFile, mkdtemp, writeFile, rm} from 'node:fs/promises';
import {spawnSync} from 'node:child_process';
import {fileURLToPath} from 'node:url';
import {join} from 'node:path';
import {tmpdir} from 'node:os';

const root = new URL('.', import.meta.url);
const compilerPath = fileURLToPath(new URL('../target/wasm32-unknown-unknown/release/atto_compiler.wasm', root));
const attoc = process.env.ATTOC || fileURLToPath(new URL('../target/debug/attoc', root));
const expectedSiteBlob = 'c4b654db16f9a0816a3c8867cc813cef5be9d6fa';
const sourcePath = fileURLToPath(new URL('./fixtures/ifnot-unless-guard-play.atto', root));
const encoder = new TextEncoder();
const decoder = new TextDecoder('utf8', {fatal: true});
const bytes = await readFile(compilerPath);
assert.ok(WebAssembly.validate(bytes), 'compiler must be real wasm32 Wasm');
const module = new WebAssembly.Module(bytes);
assert.deepEqual(WebAssembly.Module.imports(module), [], 'compiler has no ambient imports');
const {exports: api} = new WebAssembly.Instance(module, {});
assert.equal(api.abi_version(), 2, 'public compiler ABI v2');
const required = ['input_ptr','input_capacity','compile','memory','result_ptr','result_length','error_ptr','error_line_number','error_column_number'];
for (const name of required) assert.ok(name in api, 'missing browser compiler ABI export '+name);

function compileBrowser(text) {
  const input = encoder.encode(text);
  assert.ok(input.length <= api.input_capacity(), 'source exceeds browser compiler buffer');
  new Uint8Array(api.memory.buffer, api.input_ptr(), input.length).set(input);
  const rc = api.compile(input.length);
  if (rc !== 0) {
    assert.equal(api.result_length(), 0, 'failure must not expose stale guest bytes');
    const mem = new Uint8Array(api.memory.buffer);
    const start = api.error_ptr();
    let end = start;
    while (end < mem.length && end < start + 1024 && mem[end] !== 0) end++;
    const diagnostic = decoder.decode(mem.subarray(start, end));
    assert.match(diagnostic, /^E_[A-Z_]+:/, 'structured compiler diagnostic');
    assert.ok(api.error_line_number() >= 1 && api.error_column_number() >= 1, 'located diagnostic');
    return {ok:false, code:diagnostic.split(':')[0], diagnostic};
  }
  const guest = new Uint8Array(api.memory.buffer, api.result_ptr(), api.result_length()).slice();
  assert.ok(guest.length > 8, 'nonempty guest returned');
  assert.ok(WebAssembly.validate(guest), 'guest emitted by wasm32 compiler validates');
  return {ok:true, guest};
}

const cases = [
  ['return ifnot false then 7 else 9 fi;', 7n],
  ['return ifnot true then 7 else 9 fi;', 9n],
  ['return unless false then 7 else 9 fi;', 7n],
  ['return unless true then 7 else 9 fi;', 9n],
  ['do ifnot false then return 7; fi return 9;', 7n],
  ['do ifnot true then return 7; fi return 9;', 9n],
  ['do unless false then return 7; fi return 9;', 7n],
  ['do unless true then return 7; fi return 9;', 9n],
  ['ifnot false { return 7; } return 9;', 7n],
  ['ifnot true { return 7; } return 9;', 9n],
  ['unless false { return 7; } return 9;', 7n],
  ['unless true { return 7; } return 9;', 9n],
  ['do ifnot false then return 7; else return 9; fi', 7n],
  ['do unless true then return 7; else return 9; fi', 9n],
  ['unless true { return 7; } else { return 9; }', 9n],
  ['return ifnot false then 7 elif true then 8 else 9 fi;', 7n],
  ['return unless true then 7 elif true then 8 else 9 fi;', 8n],
  ['return if true then 7 else 9 fi;', 7n],
  ['do if true then return 7; fi return 9;', 7n],
  ['do if true then if false { return 7; } fi return 42;', 42n],
  ['do if true then unless false { return 42; } fi return 9;', 42n],
  ['if false { return 7; } fi return 42;', 42n],
];
const negatives = [
  ['return ifnot false then 7 fi;', 'E_VALUE_ARM'],
  ['return unless true then 7 fi;', 'E_VALUE_ARM'],
  ['return ifnot false then else 9 fi;', 'E_VALUE_ARM'],
  ['return unless true then 7 else fi;', 'E_VALUE_ARM'],
  ['ifnot false then return 7; fi return 42;', 'E_SYNTAX'],
  ['unless false then return 7; fi return 42;', 'E_SYNTAX'],
  ['return unless 5 then 7 else 9 fi;', 'E_TYPE'],
  ['return ifnot true then 7 else false fi;', 'E_TYPE'],
  ['do unless 5 then return 7; fi return 42;', 'E_TYPE'],
  ['ifnot 3 { return 7; } return 42;', 'E_TYPE'],
  ['do unless true { return 7; }', 'E_SYNTAX'],
  ['do ifnot false { return 7; }', 'E_SYNTAX'],
  ['ifnot false { return 7; } fi return 42;', 'E_SYNTAX'],
];

const temp = await mkdtemp(join(tmpdir(), 'atto-conditional-browser-'));
try {
  async function run(source, expected, label) {
    const filename = join(temp, label+'.atto');
    const output = join(temp, label+'.wasm');
    await writeFile(filename, source);
    const browser = compileBrowser(source);
    assert.equal(browser.ok, true, label+': '+browser.diagnostic);
    const cli = spawnSync(attoc, [filename,'-o',output], {encoding:'utf8', timeout:30000});
    assert.equal(cli.status, 0, label+': native compiler failed: '+cli.stderr);
    const nativeBytes = await readFile(output);
    assert.deepEqual(Buffer.from(browser.guest), nativeBytes,
      label+': native and wasm32 compiler must emit byte-identical guests');
    const guestModule = new WebAssembly.Module(browser.guest);
    assert.deepEqual(WebAssembly.Module.imports(guestModule), [],
      label+': unexpected host import');
    const guest = new WebAssembly.Instance(guestModule, {});
    assert.equal(guest.exports.main(), expected, label+': wrong guest result');
    assert.equal(guest.exports.panic_code.value, 0, label+': guest panic');
  }
  for (const [i,[body, expected]] of cases.entries()) {
    await run('pub fnc main(): int { '+body+' }\n', expected, 'case-'+i);
  }
  const siteSource = await readFile(sourcePath);
  const siteBlob = createHash('sha1')
    .update('blob '+siteSource.length+'\0').update(siteSource).digest('hex');
  assert.equal(siteBlob, expectedSiteBlob, 'website source changed without review');
  await run(decoder.decode(siteSource), 42n, 'exact-website');
  // Observe real side effects to prove the predicate is evaluated ONCE,
  // later elif conditions are not negated, and skipped arms stay lazy.
  const recorded = [
    ['fnc recorded(bool v, int marker): bool { stdio.println(marker); return v; }\n'
      +'pub fnc main(): int { return unless recorded(false, 1) then 42 elif recorded(true, 2) then 7 else 9 fi; }',
      42n, '1\n'],
    ['fnc recorded(bool v, int marker): bool { stdio.println(marker); return v; }\n'
      +'pub fnc main(): int { return ifnot recorded(true, 1) then 42 elif recorded(true, 2) then 7 else 9 fi; }',
      7n, '1\n2\n'],
    ['fnc recorded(bool v, int marker): bool { stdio.println(marker); return v; }\n'
      +'pub fnc main(): int { do ifnot recorded(false, 1) then stdio.println(3); return 42; fi return 9; }',
      42n, '1\n3\n'],
    ['fnc recorded(bool v, int marker): bool { stdio.println(marker); return v; }\n'
      +'pub fnc main(): int { unless recorded(true, 1) { return 7; } else { return 42; } }',
      42n, '1\n'],
  ];
  for (const [i,[source, expected, traceExpected]] of recorded.entries()) {
    const label = 'side-effect-'+i;
    const filename = join(temp, label+'.atto');
    const output = join(temp, label+'.wasm');
    await writeFile(filename, source);
    const compiled = compileBrowser(source);
    assert.equal(compiled.ok, true, compiled.diagnostic || label);
    const cli = spawnSync(attoc, [filename,'-o',output], {encoding:'utf8',timeout:30000});
    assert.equal(cli.status, 0, label+': '+cli.stderr);
    assert.deepEqual(Buffer.from(compiled.guest), await readFile(output), label+': native/wasm guest byte drift');
    const guestModule = new WebAssembly.Module(compiled.guest);
    const imports = WebAssembly.Module.imports(guestModule);
    assert.deepEqual(imports.map(imp => imp.module), ['atto','atto'],
      label+': only the two explicit stdout imports are allowed');
    assert.deepEqual(imports.map(imp => imp.name).sort(), ['print_i64','print_newline'],
      label+': unexpected host capability import');
    let trace = '';
    const guest = new WebAssembly.Instance(guestModule, {atto:{
      print_i64: v => {trace += String(v);},
      print_newline: () => {trace += '\n';},
    }});
    assert.equal(guest.exports.main(), expected, label+': incorrect control flow');
    assert.equal(trace, traceExpected, label+': predicate re-evaluated or arm run unexpectedly');
    assert.equal(guest.exports.panic_code.value, 0, label+': unexpected panic');
  }
  for (const [body, code] of negatives) {
    const source = 'pub fnc main(): int { '+body+' }\n';
    const reject = compileBrowser(source);
    assert.equal(reject.ok, false, body+': unexpected executable artifact');
    assert.equal(reject.code, code, body+': wrong error code');
  }
  // A failed browser compile must never poison a subsequent valid guest.
  await run('pub fnc main(): int { return 42; }\n', 42n, 'recovery');
  console.log('PASS: 22 native/wasm32 byte-identical guests, exact website 42, 4 side-effect evaluations, 13 negatives, recovery');
} finally {
  await rm(temp, {recursive:true,force:true});
}

// Exercise genuine native-compiled Atto guest Wasm. No JS semantics fallback.
import assert from 'node:assert/strict';
import {mkdtemp, writeFile, readFile, rm} from 'node:fs/promises';
import {spawnSync} from 'node:child_process';
import {tmpdir} from 'node:os';
import {join} from 'node:path';
import {fileURLToPath} from 'node:url';

const attoc = process.env.ATTOC || fileURLToPath(new URL('../target/debug/attoc', import.meta.url));
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
  ['if true do return 42; fi return 9;', 42n],
  ['if false do return 9; else return 42; fi', 42n],
  ['ifnot false do return 42; fi return 9;', 42n],
  ['ifnot true do return 9; fi return 42;', 42n],
  ['unless false do return 42; else return 9; fi', 42n],
  ['unless true do return 9; elif true do return 42; else return 7; fi', 42n],
  ['if true do unless true { return 9; } fi return 42;', 42n],
];
const tmp = await mkdtemp(join(tmpdir(), 'atto-negated-cond-'));
try {
  for (let i=0; i<cases.length; i++) {
    const [body, expected] = cases[i];
    const source = join(tmp, i+'.atto');
    const out = join(tmp, i+'.wasm');
    await writeFile(source, 'pub fnc main(): int { '+body+' }\n');
    const result = spawnSync(attoc, [source, '-o', out], {encoding:'utf8', timeout:30000});
    assert.equal(result.status, 0, body+': '+result.stderr);
    const bytes = await readFile(out);
    assert.ok(WebAssembly.validate(bytes), body+': invalid wasm');
    const module = new WebAssembly.Module(bytes);
    assert.deepEqual(WebAssembly.Module.imports(module), [], body+': ambient host import');
    const guest = new WebAssembly.Instance(module, {});
    assert.equal(guest.exports.main(), expected, body);
    assert.equal(guest.exports.panic_code.value, 0, body);
  }
  // This is byte-identical to the website syntax preview on PR #202.
  // Prevent marketing/editor admission from outrunning the real guest ABI.
  const fullSource = await readFile(new URL('./fixtures/ifnot-unless-guard-play.atto', import.meta.url), 'utf8');
  const path = join(tmp, 'website-full.atto');
  const artifact = join(tmp, 'website-full.wasm');
  await writeFile(path, fullSource);
  const compiled = spawnSync(attoc, [path, '-o', artifact], {encoding:'utf8', timeout:30000});
  assert.equal(compiled.status, 0, 'website preview cannot compile: '+compiled.stderr);
  const fullBytes = await readFile(artifact);
  assert.ok(WebAssembly.validate(fullBytes), 'website preview produced invalid Wasm');
  const fullGuest = new WebAssembly.Module(fullBytes);
  assert.deepEqual(WebAssembly.Module.imports(fullGuest), [], 'website preview needs ambient host imports');
  assert.equal(new WebAssembly.Instance(fullGuest, {}).exports.main(), 42n);
  console.log('PASS '+cases.length+' compiled conditional guests + 1 complete website preview (42)');

} finally {
  await rm(tmp, {recursive:true, force:true});
}

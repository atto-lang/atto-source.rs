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
  console.log('PASS '+cases.length+' compiled Rust→Wasm negated conditional guest executions');
} finally {
  await rm(tmp, {recursive:true, force:true});
}

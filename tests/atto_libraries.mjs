// core/alloc/std library guests (atto-spec core-alloc-std-libraries-v1):
// compile each tests/atto_libraries/*.atto with the native attoc build, run
// the guest Wasm, and check exit code, stdout and rejections.
// usage: node tests/atto_libraries.mjs [path/to/attoc]
import { readFileSync, readdirSync, mkdtempSync } from 'node:fs';
import { spawnSync } from 'node:child_process';
import { join, resolve } from 'node:path';
import { tmpdir } from 'node:os';

const here = new URL('.', import.meta.url).pathname;
const attoc = resolve(process.argv[2] || join(here, '../target/release/attoc'));
const dir = join(here, 'atto_libraries');
const BACKEND = 'rs';
const work = mkdtempSync(join(tmpdir(), 'atto-libs-'));
const decoder = new TextDecoder('utf-8', { fatal: true });

function headers(text) {
  const meta = { flags: [], errors: [] };
  for (const line of text.split('\n')) {
    if (!line.startsWith('//')) break;
    const body = line.slice(2).trim();
    let m;
    if ((m = body.match(/^flags:\s*(.*)$/))) meta.flags = m[1].split(/\s+/).filter(Boolean);
    else if ((m = body.match(/^expect-exit:\s*(-?\d+)$/))) meta.exit = Number(m[1]);
    else if ((m = body.match(/^expect-stdout-json:\s*(.*)$/))) meta.stdout = JSON.parse(m[1]);
    else if (body === 'expect-reject') meta.reject = true;
    else if ((m = body.match(/^expect-error:\s*(.*)$/))) meta.errors.push(m[1]);
    else if ((m = body.match(new RegExp(`^expect-error-${BACKEND}:\\s*(.*)$`)))) meta.errors.push(m[1]);
  }
  return meta;
}

function runGuest(wasm) {
  const module = new WebAssembly.Module(wasm);
  let out = '';
  let instance;
  const atto = {
    print_i64: v => { out += String(v); },
    print_bool: v => { out += v ? 'true' : 'false'; },
    print_str: (p, l) => { out += decoder.decode(new Uint8Array(instance.exports.memory.buffer, p, l)); },
    print_newline: () => { out += '\n'; },
    print_f64: v => { out += String(v); },
  };
  for (const imp of WebAssembly.Module.imports(module)) {
    if (imp.module !== 'atto' || !(imp.name in atto)) throw new Error(`unexpected import ${imp.module}.${imp.name}`);
  }
  instance = new WebAssembly.Instance(module, { atto });
  return { exit: Number(instance.exports.main()), stdout: out };
}

const failures = [];
const cases = readdirSync(dir).filter(n => n.endsWith('.atto')).sort();
for (const name of cases) {
  const meta = headers(readFileSync(join(dir, name), 'utf8'));
  const out = join(work, name.replace(/\.atto$/, '.wasm'));
  const compiled = spawnSync(attoc, [...meta.flags, name, '-o', out], { cwd: dir, encoding: 'utf8' });
  if (meta.reject) {
    const bad = meta.errors.find(e => !compiled.stderr.includes(e));
    if (compiled.status === 0) failures.push(`${name}: expected rejection, compiled`);
    else if (bad) failures.push(`${name}: diagnostic lacks ${JSON.stringify(bad)}: ${compiled.stderr.trim()}`);
    else console.log(`PASS ${name} (rejected)`);
    continue;
  }
  if (compiled.status !== 0) { failures.push(`${name}: compile failed: ${compiled.stderr.trim()}`); continue; }
  let ran;
  try { ran = runGuest(readFileSync(out)); } catch (e) { failures.push(`${name}: trapped: ${e}`); continue; }
  if (ran.exit !== (meta.exit ?? 0)) failures.push(`${name}: exit ${ran.exit} != ${meta.exit ?? 0}`);
  else if (meta.stdout !== undefined && ran.stdout !== meta.stdout)
    failures.push(`${name}: stdout ${JSON.stringify(ran.stdout)} != ${JSON.stringify(meta.stdout)}`);
  else console.log(`PASS ${name} (exit ${ran.exit})`);
}
for (const f of failures) console.log(`FAIL ${f}`);
console.log(`ATTO_LIBRARIES_V1 ${cases.length - failures.length}/${cases.length} passed`);
process.exit(failures.length ? 1 : 0);

// Real native attoc -> compiled guest Wasm -> physical memory, across 3 layouts.
// This is deliberately not a host-only planner assertion or Array interpreter.
import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { spawnSync } from 'node:child_process';
import { existsSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { tmpdir } from 'node:os';
import { fileURLToPath } from 'node:url';

const here = dirname(fileURLToPath(import.meta.url));
const attoc = process.env.ATTOC ?? resolve(here, '../target/debug/attoc');
assert.ok(existsSync(attoc), 'real native attoc compiler is required: ' + attoc);
const sha256 = bytes => createHash('sha256').update(bytes).digest('hex');

const programs = [
  {
    name: 'soa-15',
    filename: 'collection-layout-soa-15.atto',
    expectedReturn: 15n,
    expectedWords: [15n, 30n, 20n, 40n, 1n, 2n],
  },
  {
    name: 'aos-15',
    filename: 'collection-layout-aos-15.atto',
    expectedReturn: 15n,
    expectedWords: [15n, 20n, 1n, 30n, 40n, 2n],
  },
  {
    name: 'aosoa-tail-5',
    filename: 'collection-layout-aosoa-tail-5.atto',
    expectedReturn: 5n,
    expectedWords: [
      1n, 2n, 3n, 4n, 10n, 20n, 30n, 40n, 100n, 200n, 300n, 400n,
      5n, 0n, 0n, 0n, 50n, 0n, 0n, 0n, 500n, 0n, 0n, 0n,
    ],
  },
];

const run = (input, output) => spawnSync(attoc, [input, '-o', output], {
  encoding: 'utf8', timeout: 30_000, maxBuffer: 1 << 20,
});
const temp = mkdtempSync(join(tmpdir(), 'atto-real-layout-guests-'));
const receipts = [];
try {
  for (const program of programs) {
    const sourcePath = resolve(here, 'fixtures/pass', program.filename);
    const wasmPath = join(temp, program.name + '.wasm');
    const compiled = run(sourcePath, wasmPath);
    assert.equal(compiled.error, undefined, program.name + ': compiler process error');
    assert.equal(compiled.status, 0, program.name + ': compiler rejected source: ' + compiled.stderr);
    assert.ok(existsSync(wasmPath), program.name + ': no guest module emitted');
    const wasm = readFileSync(wasmPath);
    assert.ok(WebAssembly.validate(wasm), program.name + ': invalid guest Wasm');
    const mod = new WebAssembly.Module(wasm);
    assert.deepEqual(WebAssembly.Module.imports(mod), [], program.name + ': host-side storage import prohibited');
    const instance = new WebAssembly.Instance(mod, {});
    assert.equal(typeof instance.exports.main, 'function', program.name + ': missing real main export');
    const value = instance.exports.main();
    assert.equal(value, program.expectedReturn, program.name + ': wrong compiled guest result');
    assert.equal(instance.exports.panic_code.value, 0, program.name + ': unexpected guest panic');
    const memory = instance.exports.memory;
    assert.ok(memory instanceof WebAssembly.Memory, program.name + ': no guest linear memory');
    const view = new DataView(memory.buffer);
    const matchOffsets = [];
    for (let offset = 0; offset + program.expectedWords.length * 8 <= view.byteLength; offset += 8) {
      if (program.expectedWords.every((n, i) => view.getBigInt64(offset + i * 8, true) === n)) {
        matchOffsets.push(offset);
      }
    }
    assert.equal(matchOffsets.length, 1, program.name + ': exact physical layout not uniquely found in guest memory');
    // A result alone is insufficient. For SoA, the complete i64 physical
    // columns must be contiguous, with no AoS row-major masquerading.
    if (program.name === 'soa-15') {
      assert.notDeepEqual(program.expectedWords, [15n, 20n, 1n, 30n, 40n, 2n]);
    }
    receipts.push({
      kind: 'SOURCE_COMPILED_WASM_GUEST',
      fixture: program.filename,
      source_sha256: sha256(readFileSync(sourcePath)),
      wasm_sha256: sha256(wasm),
      return_value: String(value),
      physical_words: program.expectedWords.map(String),
      guest_memory_byte_offset: matchOffsets[0],
      imports: 0,
      panic_code: 0,
      guest_executed: true,
    });
  }

  // Prove fail-closed compiler negatives with no generated guest artifact.
  const milestone = readFileSync(resolve(here, 'fixtures/pass/collection-layout-soa-15.atto'), 'utf8');
  const invalid = [
    ['wrong-layout', milestone.replace('layout=soa', 'layout=bogus'), 'E_LAYOUT_VALUE'],
    ['duplicate-layout', milestone.replace('layout=soa', 'layout=soa, layout=aos'), 'E_GENERIC_DUP'],
    ['aosoa-zero', milestone.replace('layout=soa', 'layout=aosoa(0)'), 'E_LAYOUT_SIZE'],
    ['aosoa-dynamic', milestone.replace('layout=soa', 'layout=aosoa(n)'), 'E_LAYOUT_SIZE'],
    ['wrong-element', milestone.replace('Array<Particle,', 'Array<Unknown,'), 'E_UNKNOWN_TYPE'],
    ['out-of-bounds-row', milestone.replace('particles[0].x += 5', 'particles[2].x += 5'), 'E_LAYOUT_BOUNDS'],
    ['out-of-bounds-column', milestone.replace('columns.x[0]', 'columns.x[2]'), 'E_LAYOUT_BOUNDS'],
    ['duplicate-field', milestone.replace('Particle(x=10, y=20, velocity=1)', 'Particle(x=10, x=20, velocity=1)'), 'E_STRUCT_FIELD'],
    ['unverified-mutable-view', milestone.replace('particles[0].x += 5;', 'let aliased = particles.columns.x;'), 'E_UNSUPPORTED'],
  ];
  for (const [name, source, diagnostic] of invalid) {
    const input = join(temp, name + '.atto');
    const output = join(temp, name + '.wasm');
    writeFileSync(input, source);
    const compiled = run(input, output);
    assert.equal(compiled.error, undefined, name + ': compiler process error');
    assert.equal(compiled.status, 1, name + ': invalid source was accepted');
    assert.ok(compiled.stderr.includes(diagnostic), name + ': expected ' + diagnostic + ', got: ' + compiled.stderr);
    assert.equal(existsSync(output), false, name + ': invalid source emitted a Wasm module');
    receipts.push({ kind: 'COMPILER_NEGATIVE', case: name, source_sha256: sha256(source),
      diagnostic, no_artifact: true });
  }

  console.log(JSON.stringify({
    schema: 'atto.collection-layout.guest-evidence.v1',
    compiler_binary_sha256: sha256(readFileSync(attoc)),
    compiler_source_head_claim: process.env.ATTO_COMPILER_SOURCE_SHA ?? null,
    node: process.version,
    results: receipts,
  }, null, 2));
} finally {
  rmSync(temp, { recursive: true, force: true });
}

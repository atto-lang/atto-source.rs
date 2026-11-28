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
    name: 'seeded-make-aosoa-9',
    filename: 'collection-layout-seeded-make-aosoa-9.atto',
    expectedReturn: 9n,
    expectedWords: [1n, 2n, 3n, 10n, 20n, 30n, 9n, 0n, 0n, 40n, 0n, 0n],
  },
  {
    name: 'seeded-make-soa-15',
    filename: 'collection-layout-seeded-make-soa-15.atto',
    expectedReturn: 15n,
    expectedWords: [15n, 30n, 20n, 40n, 1n, 2n],
  },
  {
    name: 'contextual-make-soa-15',
    filename: 'collection-layout-contextual-make-soa-15.atto',
    expectedReturn: 15n,
    expectedWords: [15n, 30n, 20n, 40n, 1n, 2n],
  },
  {
    name: 'fixed-size-soa-15',
    filename: 'collection-layout-fixed-size-soa-15.atto',
    expectedReturn: 15n,
    expectedWords: [15n, 30n, 20n, 40n, 1n, 2n],
  },
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

  // Both lexical projections must be independently proven by compiled guests.
  const seededProjection = readFileSync(resolve(here, 'fixtures/pass/collection-layout-seeded-make-soa-15.atto'), 'utf8');
  for (const [alias, expected] of [['X', 2n], ['Base', 5n]]) {
    const source = seededProjection.replace('return particles.columns.x[0];', `return ${alias}.size;`);
    const input = join(temp, `project-${alias}.atto`);
    const output = join(temp, `project-${alias}.wasm`);
    writeFileSync(input, source);
    const compiled = run(input, output);
    assert.equal(compiled.error, undefined);
    assert.equal(compiled.status, 0, compiled.stderr);
    const wasm = readFileSync(output);
    assert.ok(WebAssembly.validate(wasm));
    const mod = new WebAssembly.Module(wasm);
    assert.deepEqual(WebAssembly.Module.imports(mod), []);
    const instance = new WebAssembly.Instance(mod, {});
    assert.equal(instance.exports.main(), expected);
    assert.equal(instance.exports.panic_code.value, 0);
    receipts.push({kind: 'SOURCE_COMPILED_WASM_GUEST', case: `project-${alias}`,
      source_sha256: sha256(source), wasm_sha256: sha256(wasm), return_value: String(expected), guest_executed: true});
  }

  // Prove fail-closed compiler negatives with no generated guest artifact.
  const milestone = readFileSync(resolve(here, 'fixtures/pass/collection-layout-soa-15.atto'), 'utf8');
  const fixed = readFileSync(resolve(here, 'fixtures/pass/collection-layout-fixed-size-soa-15.atto'), 'utf8');
  const contextual = readFileSync(resolve(here, 'fixtures/pass/collection-layout-contextual-make-soa-15.atto'), 'utf8');
  const seeded = readFileSync(resolve(here, 'fixtures/pass/collection-layout-seeded-make-soa-15.atto'), 'utf8');
  const tiled = readFileSync(resolve(here, 'fixtures/pass/collection-layout-seeded-make-aosoa-9.atto'), 'utf8');
  const invalid = [
    ['seed-aosoa-zero', tiled.replace('aosoa(3)', 'aosoa(0)'), 'E_LAYOUT_SIZE'],
    ['seed-aosoa-over-limit', tiled.replace('aosoa(3)', 'aosoa(1025)'), 'E_LAYOUT_SIZE'],
    ['seed-aosoa-dynamic', tiled.replace('aosoa(3)', 'aosoa(n)'), 'E_LAYOUT_SIZE'],
    ['seed-aosoa-tail-mutation', tiled.replace('particles[3]', 'particles[4]'), 'E_LAYOUT_BOUNDS'],
    ['seed-aosoa-tail-read', tiled.replace('columns.x[3]', 'columns.x[4]'), 'E_LAYOUT_BOUNDS'],
    ['seed-size-inherited-mismatch', seeded.replace(' with {size=infer}', ''), 'E_ARRAY_SIZE_MISMATCH'],
    ['seed-size-invalid', seeded.replace('size=5', 'size=-1'), 'E_LAYOUT_SIZE'],
    ['seed-wrong-nominal', seeded.replace('T=Particle', 'T=Other'), 'E_UNKNOWN_TYPE'],
    ['seed-duplicate-slot', seeded.replace('size=5', 'size=5,size=5'), 'E_GENERIC_DUP'],
    ['seed-capture-collision', seeded.replace('infer X from Base', 'infer Base from Base'), 'E_GENERIC_DUP'],
    ['seed-unknown-source', seeded.replace('from Base>', 'from Unknown>'), 'E_LAYOUT_SYNTAX'],
    ['seed-new-inline', seeded.replace('make {', 'new {'), 'E_CONSTRUCTION_CATEGORY'],
    ['contextual-new-in-inline-slot', contextual.replace('make {', 'new {'), 'E_CONSTRUCTION_CATEGORY'],
    ['contextual-make-missing', contextual.replace('x:10, y:20, velocity:1', 'x:10, y:20'), 'E_STRUCT_FIELD'],
    ['contextual-make-duplicate', contextual.replace('x:10, y:20, velocity:1', 'x:10, x:20, velocity:1'), 'E_STRUCT_FIELD'],
    ['contextual-make-extra', contextual.replace('x:10, y:20, velocity:1', 'x:10, y:20, unknown:1'), 'E_STRUCT_FIELD'],
    ['fixed-size-too-small', fixed.replace('size=2', 'size=1'), 'E_ARRAY_SIZE_MISMATCH'],
    ['fixed-size-too-large', fixed.replace('size=2', 'size=3'), 'E_ARRAY_SIZE_MISMATCH'],
    ['fixed-size-negative', fixed.replace('size=2', 'size=-1'), 'E_LAYOUT_SIZE'],
    ['fixed-size-over-limit', fixed.replace('size=2', 'size=1025'), 'E_LAYOUT_SIZE'],
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

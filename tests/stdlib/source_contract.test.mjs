// Source-level consistency audit. This is NOT an Atto compiler or runtime test.
// Requires node >= 18; operates on checked-out source, not generated snapshots.
import test from "node:test";
import assert from "node:assert/strict";
import {readFileSync} from "node:fs";
import {fileURLToPath} from "node:url";
import {resolve,dirname} from "node:path";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "../..");
const names = ["websocket", "quic", "compute"];
const sources = Object.fromEntries(names.map(name => [
  name, readFileSync(resolve(root, "stdlib/std", name + ".atto"), "utf8")
]));
const guest = readFileSync(resolve(root, "tests/stdlib/websocket_quic_compute_smoke.atto"), "utf8");
const boundary = readFileSync(resolve(root, "tests/stdlib/protocol_boundaries.atto"), "utf8");

function verify(source, fixture) {
  const declared = Object.fromEntries(names.map(name => [
    name, new Set([...source[name].matchAll(/\bpub\s+fnc\s+([a-z][a-z0-9_]*)\s*\(/g)].map(m => m[1]))
  ]));
  assert.equal(names.reduce((n, name) => n + declared[name].size, 0), 42,
    "unexpected public Atto API count");
  for (const name of names) {
    assert.ok(source[name].includes("pub fnc "), name + " has no public Atto functions");
    // Every helper call in a module must resolve to a function in the same module.
    for (const match of source[name].matchAll(new RegExp("\\b(" + name + "_[a-z0-9_]+)\\s*\\(", "g"))) {
      assert.ok(declared[name].has(match[1]), "unbound internal helper: " + match[1]);
    }
  }
  const isModule = /^\s*import\s+module\s+/m.test(fixture);
  if (isModule) {
    for (const name of names) {
      assert.match(source[name], new RegExp("\\bdefine pub module " + name + " as\\b"));
      assert.ok(fixture.includes('import module ' + name +
        ' from "+stdlib/std/' + name + '.atto";'), "module import missing: " + name);
    }
    for (const match of fixture.matchAll(/(?<![\w.])(?:websocket|quic|compute)_[a-z0-9_]+\s*\(/g)) {
      assert.fail("unqualified guest call bypasses required module import: " + match[0]);
    }
    // The Rust/Wasm frontend deliberately rejects '%' until its checked scalar
    // remainder semantics are lowered. Each backend must own a valid source
    // implementation rather than assuming another backend's operator set.
    for (const name of names) {
      assert.doesNotMatch(source[name], /%/, name + " requires unsupported remainder operator");
      const withoutDecls = source[name].replace(
        /\bpub\s+fnc\s+[a-z][a-z0-9_]*\s*\(/g, "pub fnc (");
      const bare = new RegExp("(?<![\\w.])" + name + "_[a-z0-9_]+\\s*\\(", "g");
      assert.equal([...withoutDecls.matchAll(bare)].length, 0,
        name + " must explicitly qualify same-module function calls");
    }
  } else {
    const imports = {};
    for (const match of fixture.matchAll(/import fnc ([\s\S]*?) from '\.\.\/\.\.\/stdlib\/std\/([a-z]+)';/g)) {
      assert.ok(names.includes(match[2]), "unexpected import " + match[2]);
      assert.equal(imports[match[2]], undefined, "duplicate import " + match[2]);
      imports[match[2]] = new Set(match[1].split(",").map(v => v.trim()));
    }
    for (const name of names) {
      assert.ok(imports[name], "missing imported group " + name);
      for (const fn of imports[name]) {
        assert.ok(declared[name].has(fn), "imported but unexported: " + name + "." + fn);
      }
    }
    for (const match of fixture.matchAll(/\b(?:websocket|quic|compute)_[a-z0-9_]+\s*\(/g)) {
      const fn = match[0].split("(")[0].trim();
      const name = names.find(n => fn.startsWith(n + "_"));
      assert.ok(declared[name].has(fn), "undefined guest callee " + fn);
      assert.ok(imports[name].has(fn), "callee not imported " + fn);
    }
  }
  // Guest-qualified callsites are checked against the public module exports.
  for (const match of fixture.matchAll(/\b(websocket|quic|compute)\.([a-z_][a-z0-9_]*)\s*\(/g)) {
    assert.ok(declared[match[1]].has(match[2]), "undefined qualified guest callee " + match[0]);
  }
  const codes = [...fixture.matchAll(/\bthen return ([0-9]+); fi/g)].map(m => Number(m[1]));
  assert.deepEqual(codes, Array.from({length:37}, (_,i) => i + 1), "37 sequential guest cases required");
  assert.match(fixture, /\breturn 0;\s*\}\s*$/, "guest success return absent");
  assert.doesNotMatch(fixture, /\bquic_varint_min_width\s*\(/,
    "guest references obsolete unbound QUIC helper");
}

test("source exports and every guest call resolve; all 37 guest cases accounted for", () => {
  verify(sources, guest);
});
test("unexported helper or unlinked callee is detected (mutation)", () => {
  const modified = {...sources,
    quic: sources.quic.replace("pub fnc quic_encoding_min_width", "fnc quic_encoding_min_width")};
  assert.throws(() => verify(modified, guest));
  assert.throws(() => verify(sources, guest.replace(
    /quic(?:\.)?quic_encoding_min_width|quic_encoding_min_width/,
    "quic_bogus_min_width")));
});
test("duplicate numbered cases and missing imports cannot pass (mutation)", () => {
  assert.throws(() => verify(sources, guest.replace("then return 37; fi", "then return 36; fi")));
  assert.throws(() => verify(sources, guest.replace(/^import module compute.*\n/m, "")
    .replace(/import fnc compute_exact_buffer_layout,[\s\S]*? from '\.\.\/\.\.\/stdlib\/std\/compute';\n/, "")));
});


function verifyBoundary(fixture, ownedSources) {
  const names = ["websocket", "quic", "compute"];
  const declared = names.flatMap(name => [...ownedSources[name].matchAll(
    /\bpub\s+fnc\s+([a-z][a-z0-9_]*)\s*\(/g
  )].map(match => name + "." + match[1]));
  assert.equal(declared.length, 42, "boundary coverage must match all public Atto exports");
  for (const name of names) {
    assert.ok(fixture.includes('import module ' + name + ' from "+stdlib/std/' + name + '.atto";'),
      "missing native source module in boundary guest: " + name);
  }
  for (const qualified of declared) {
    assert.ok(fixture.includes(qualified + "("),
      "no native Atto boundary case calls exported function: " + qualified);
  }
  for (const match of fixture.matchAll(/\b(websocket|quic|compute)\.([a-z_][a-z0-9_]*)\s*\(/g)) {
    assert.ok(declared.includes(match[1] + "." + match[2]),
      "unexported Atto module member in boundary guest: " + match[0]);
  }
  const numbered = [...fixture.matchAll(/\bthen return (\d+); fi/g)].map(x => Number(x[1]));
  assert.deepEqual(numbered, Array.from({length: 109}, (_, i) => i + 1),
    "boundary guest must contain 109 uniquely numbered cases in sequence");
  assert.match(fixture, /\breturn 0;\s*\}\s*$/,
    "boundary guest must return zero only after all checks");
  return true;
}

test("109 independently compiled Atto boundaries cover all 42 public functions", () => {
  assert.equal(verifyBoundary(boundary, sources), true);
});

test("boundary auditor rejects missing imports, callsites, exports and bad case numbers", () => {
  assert.throws(() => verifyBoundary(
    boundary.replace('import module compute from "+stdlib/std/compute.atto";', ""), sources));
  assert.throws(() => verifyBoundary(
    boundary.replace("quic.quic_stream_id_valid(", "quic.quic_unknown_id("), sources));
  assert.throws(() => verifyBoundary(
    boundary.replace("then return 109; fi", "then return 108; fi"), sources));
  assert.throws(() => verifyBoundary(
    boundary.replace("compute.compute_upload_admit(", "compute.compute_typo("), sources));
  const removedExport = {...sources,
    websocket: sources.websocket.replace("pub fnc websocket_mask_octet", "fnc websocket_mask_octet")};
  assert.throws(() => verifyBoundary(boundary, removedExport));
});


test("bounded nonnegative i64 quotient/subtract identity matches remainder arithmetic", () => {
  const max = (1n << 63n) - 1n;
  const divisors = [1n, 2n, 4n, 8n, 16n, 255n, 256n, 65535n,
    (1n << 62n), max];
  const values = [0n, 1n, 2n, 3n, 7n, 8n, 63n, 64n, 125n, 126n,
    255n, 256n, 15293n, 16384n, (1n << 62n)-1n, (1n << 62n), max-1n, max];
  let state = 0x123456789abcdefn;
  for (let i = 0; i < 2048; i++) {
    state = (state * 6364136223846793005n + 1442695040888963407n) & max;
    values.push(state);
  }
  for (const value of values) for (const divisor of divisors) {
    const product = (value / divisor) * divisor;
    assert.ok(product >= 0n && product <= value && product <= max,
      "intermediate product must fit signed i64 and never exceed input");
    assert.equal(value - product, value % divisor,
      "backend-owned quotient/subtract expression changed remainder semantics");
  }
});

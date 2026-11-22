// Independent actual compiled .atto guest execution. Never model source rules in JS.
import test from "node:test";
import assert from "node:assert/strict";
import {readFileSync} from "node:fs";

const [successPath, mutationPath, ...other] = process.argv.slice(2);
assert.ok(successPath && mutationPath && other.length === 0,
  "expected normal.wasm and mutation.wasm from native Atto source compiler");

async function invoke(path) {
  const bytes = readFileSync(path);
  assert.ok(bytes.length > 8, "missing compiled guest");
  assert.equal(WebAssembly.validate(bytes), true, "invalid native compiler output");
  const module = new WebAssembly.Module(bytes);
  assert.deepEqual(WebAssembly.Module.imports(module), [],
    "pure Atto protocol guest cannot use a host callback");
  const instance = await WebAssembly.instantiate(module, {});
  assert.equal(typeof instance.exports.main, "function", "no native guest main");
  const result = instance.exports.main();
  assert.equal(typeof result, "bigint", "expected signed 64-bit Atto main return");
  return result;
}

test("all 37 checks execute inside real source-compiled Atto Wasm", async () => {
  assert.equal(await invoke(successPath), 0n);
});

test("mutating QUIC expected byte fails at exact native guest assertion 12", async () => {
  assert.equal(await invoke(mutationPath), 12n,
    "a compiled assertion mutation must be observed in the executing guest");
});

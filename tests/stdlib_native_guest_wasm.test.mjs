// Actual compiled Atto Wasm guest verification, independent of Rust host models.
import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";

const [wasmPath, ...other] = process.argv.slice(2);
assert.ok(wasmPath && other.length === 0, "expected exactly one compiled guest file");

test("source-linked std modules execute 37 Atto assertions in WebAssembly", async () => {
  const bytes = readFileSync(wasmPath);
  assert.ok(bytes.length > 8, "empty or truncated compiled guest");
  assert.equal(WebAssembly.validate(bytes), true, "not valid guest Wasm");
  const module = new WebAssembly.Module(bytes);
  const imports = WebAssembly.Module.imports(module);
  assert.deepEqual(imports, [], "pure stdlib guest must not import host callbacks");
  const instance = await WebAssembly.instantiate(module, {});
  assert.equal(typeof instance.exports.main, "function", "missing guest main");
  const observed = instance.exports.main();
  assert.equal(typeof observed, "bigint", "expected true 64-bit Atto main return");
  assert.equal(observed, 0n, "one of the 37 Atto protocol/compute assertions failed");
});

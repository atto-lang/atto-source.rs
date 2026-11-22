// Independent real guest execution: printing 49 is observable host output,
// not a comment, static token match, or inferred result from a Rust test.
import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";

const [path, ...extra] = process.argv.slice(2);
assert.ok(path && extra.length === 0, "expected one compiled Wasm guest path");

test("use namespace storefront; import module math; prints 49 and returns 0", async () => {
  const bytes = readFileSync(path);
  assert.equal(WebAssembly.validate(bytes), true, "compiler must emit real Wasm");
  const module = new WebAssembly.Module(bytes);
  const imports = WebAssembly.Module.imports(module)
    .map(({ module, name }) => `${module}.${name}`)
    .sort();
  assert.deepEqual(imports, ["atto.print_i64", "atto.print_newline"]);
  const stdout = [];
  // With a precompiled WebAssembly.Module, instantiate returns Instance directly.
  // The {instance, module} wrapper belongs to the byte-buffer overload only.
  const instance = await WebAssembly.instantiate(module, {
    atto: {
      print_i64: value => stdout.push(value.toString()),
      print_newline: () => stdout.push("\n"),
    },
  });
  assert.equal(typeof instance.exports.main, "function");
  assert.equal(instance.exports.main(), 0n);
  assert.equal(stdout.join(""), "49\n", "actual guest must print 49 and newline");
  assert.equal(Object.hasOwn(instance.exports, "math"), false,
    "language module is not an unguarded Wasm export");
  assert.equal(Object.hasOwn(instance.exports, "square"), false,
    "imported module members are not unguarded Wasm exports");
});

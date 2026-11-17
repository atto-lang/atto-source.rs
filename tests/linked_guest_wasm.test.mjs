// Real Node WebAssembly guest execution, independent of the Rust AST/typechecker.
import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";

const paths = process.argv.slice(2);
assert.equal(paths.length, 2, "two linked wasm guest paths required");

for (const [i, path] of paths.entries()) {
  test(i === 0 ? "cross-package public module executes in linked guest" :
    "same-package pub[pkg] module executes in linked guest", async () => {
    const bytes = readFileSync(path);
    assert.equal(WebAssembly.validate(bytes), true);
    const imports = WebAssembly.Module.imports(new WebAssembly.Module(bytes));
    assert.deepEqual(imports, [], "pure linked guest may not acquire host imports");
    const { instance } = await WebAssembly.instantiate(bytes, {});
    assert.equal(typeof instance.exports.main, "function");
    assert.equal(instance.exports.main(), 42n);
    assert.equal(Object.hasOwn(instance.exports, "Math"), false,
      "language module must never become an unguarded Wasm host export");
    assert.equal(Object.hasOwn(instance.exports, "secret"), false,
      "private members must never become Wasm host exports");
  });
}

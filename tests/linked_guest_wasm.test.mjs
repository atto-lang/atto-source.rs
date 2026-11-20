// Real Node WebAssembly guest execution, independent of the Rust AST/typechecker.
import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";

const paths = process.argv.slice(2);
assert.equal(paths.length, 4, "four linked wasm guest paths required");

for (const [i, path] of paths.entries()) {
  const names = [
    "cross-package public module executes in linked guest",
    "same-package pub[pkg] module executes in linked guest",
    "three-file chained namespace imports execute",
    "filesystem manifest-rooted linked package executes",
  ];
  test(names[i], async () => {
    const bytes = readFileSync(path);
    assert.equal(WebAssembly.validate(bytes), true);
    const imports = WebAssembly.Module.imports(new WebAssembly.Module(bytes));
    assert.deepEqual(imports, [], "pure linked guest may not acquire host imports");
    assert.equal(
      WebAssembly.Module.customSections(new WebAssembly.Module(bytes), "atto.exports.modules.v1").length,
      0,
      "an imported module must not be re-exported in language-level metadata",
    );
    const { instance } = await WebAssembly.instantiate(bytes, {});
    assert.equal(typeof instance.exports.main, "function");
    assert.equal(instance.exports.main(), 42n);
    assert.equal(Object.hasOwn(instance.exports, "Math"), false,
      "language module must never become an unguarded Wasm host export");
    assert.equal(Object.hasOwn(instance.exports, "secret"), false,
      "private members must never become Wasm host exports");
  });
}

// Verify Cargo's explicit integration-test inventory without relying on a Rust runner.
// This is a structural gate, NOT evidence that cargo test, clippy or guest Wasm ran.
import assert from 'node:assert/strict';
import {readFileSync, readdirSync, existsSync} from 'node:fs';
import {fileURLToPath} from 'node:url';
import {dirname, join} from 'node:path';

const root = join(dirname(fileURLToPath(import.meta.url)), '..');
const manifest = readFileSync(join(root, 'Cargo.toml'), 'utf8');
const nativeManifest = readFileSync(join(root, 'native/Cargo.toml'), 'utf8');

// These tests import oreslang_source_rs or invoke the native-only oresc CLI.
// They MUST NOT be built against atto_compiler. Their continued presence is
// tracked explicitly rather than silently discarded. The separate native
// package currently runs its own native/tests suite; porting these 38 legacy
// root suites into that package remains a separate coverage obligation.
const nativeRootTests = new Set([
  "actor_arena_contract",
  "aosoa_checked_addressing",
  "atto_immutable_parity",
  "atto_match_tilde_failclosed_v1",
  "atto_scalar_java_llvm_v1",
  "checked_i64_cli",
  "checked_i64_interprocedural",
  "checked_i64_native",
  "checked_i64_ref_layout",
  "collection_identity_failclosed",
  "compiler_boundary_matrix",
  "compiler_numeric_separators_batch4",
  "conformance",
  "declarative_export_trailing_boundary",
  "declarative_exports",
  "effect_interop",
  "execution_domain_admission",
  "flow_contract",
  "http_actor_admission",
  "immutable_binding_parity",
  "layout_plan",
  "layout_trace",
  "name_first_formals",
  "native_object_execution",
  "oresllm_native_corpus",
  "output_publication",
  "owner_arena",
  "platform_core_v1",
  "profile_cli",
  "profile_cli_hardening",
  "pull_push_unsupported_gate",
  "singleton_admission",
  "source_byte_limit_cli",
  "source_i64_parity",
  "strict_aot_negative_cli",
  "tiering_policy",
  "trace_sites",
  "unary_plus_parity"
]);

assert.match(manifest, /^autobins\s*=\s*false\s*$/m);
assert.match(manifest, /^autotests\s*=\s*false\s*$/m);
assert.match(nativeManifest, /^name\s*=\s*"oreslang-source-rs"\s*$/m);
assert.ok(existsSync(join(root, 'native/src/lib.rs')), 'native crate missing');
const disk = readdirSync(join(root, 'tests'))
  .filter(name => name.endsWith('.rs')).map(name => name.slice(0, -3));
const diskSet = new Set(disk);
assert.equal(diskSet.size, disk.length, 'duplicate root Rust test basename');
for (const name of nativeRootTests) {
  assert.ok(diskSet.has(name), 'native-exclusion entry missing: ' + name);
}
const expected = disk.filter(name => !nativeRootTests.has(name)).sort();
const pattern = /\[\[test\]\]\s*name\s*=\s*"([^"]+)"\s*path\s*=\s*"tests\/([^"]+)\.rs"/g;
const parsed = [...manifest.matchAll(pattern)];
assert.equal((manifest.match(/\[\[test\]\]/g) ?? []).length, parsed.length,
  'test target missing explicit name and path');
const actual = parsed.map(m => {
  assert.equal(m[1], m[2], 'test target path/name mismatch: ' + m[1]);
  return m[1];
});
assert.equal(new Set(actual).size, actual.length, 'duplicate explicit browser test target');
assert.deepEqual(actual.sort(), expected,
  'Cargo browser test targets must cover every non-native root Rust test');
assert.equal(nativeRootTests.size, 38, 'unexpected native legacy test inventory');
console.log('BROWSER_CARGO_TARGETS_AUDITED: ' + actual.length +
  ' browser tests; ' + nativeRootTests.size +
  ' legacy native-only root tests explicitly excluded (not executed by this gate)');

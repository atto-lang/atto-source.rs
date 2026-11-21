// Executes exact-source, SHA-1 Git-blob-verified Atto NIC logic through
// the real Rust attoc -> Wasm path. Node runs generated Atto guests, not a
// JavaScript/Rust replacement for the virtual device.
// No Python, no network, no git credentials, no runtime compilation fallback.
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { mkdtempSync, readFileSync, writeFileSync, existsSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const here = fileURLToPath(new URL('.', import.meta.url));
const repo = resolve(here, '../../..');
const compiler = resolve(process.env.ATTOC || join(repo, 'target/release/attoc'));
const manifest = JSON.parse(readFileSync(join(here, 'nic/snapshot.json'), 'utf8'));
const gitBlob = bytes => createHash('sha1')
  .update(Buffer.concat([Buffer.from('blob ' + bytes.length + String.fromCharCode(0)), bytes]))
  .digest('hex');
const sha256 = bytes => createHash('sha256').update(bytes).digest('hex');
const fixture = [];
for (const item of manifest.files) {
  const bytes = readFileSync(join(repo, item.copy));
  assert.equal(gitBlob(bytes), item.git_blob_sha1,
    'Atto source modified or not byte-identical: ' + item.copy);
  fixture.push(bytes.toString('utf8'));
}
// The browser compiler still refuses int/int division under language issue
// S-25. Compile only whole, unmodified function definitions that do not use
// that undecided operator; keep the source file blob SHA-verified above.
const admitted = new Set([
  'valid_mac', 'valid_vlan_id', 'valid_ethernet_frame',
  'valid_descriptor_ring', 'can_receive', 'can_transmit',
  'next_ring_slot', 'available_descriptors',
  'vm_lease_matches', 'valid_guest_range', 'supported_split_ring_size',
  'split_ring_pending', 'split_ring_can_consume',
  'valid_flat_descriptor', 'can_start_fast_path_batch',
  'valid_virtual_ethernet_frame', 'virtio_net_header_supported',
  'nic_can_consume_rx', 'nic_can_handoff_switch',
  'next_u16_queue_index', 'queue_can_offer', 'queue_can_claim', 'next_descriptor_state', 'queue_can_claim_flat', 'queue_next_used_after_complete',
]);
const compiledFunctions = [];
const excluded = [];
const extracted = [];
for (const source of fixture) {
  const marks = [...source.matchAll(new RegExp('^pub fnc ([a-z_][a-z_0-9]*)[(]', 'gm'))];
  assert.ok(marks.length > 0, 'no top-level Atto functions found in pinned source');
  for (let i = 0; i < marks.length; i++) {
    const name = marks[i][1];
    const start = marks[i].index;
    const end = i + 1 < marks.length ? marks[i + 1].index : source.length;
    const exactSourceRange = source.slice(start, end).trimEnd();
    if (admitted.has(name)) {
      extracted.push(exactSourceRange);
      compiledFunctions.push(name);
    } else {
      excluded.push(name);
    }
  }
}
assert.deepEqual(excluded.sort(), ['mac_is_group','valid_mmio_window']);
assert.deepEqual([...compiledFunctions].sort(), [...admitted].sort());
const fullSource = extracted.join('\n\n') + '\n\n';
// Each condition is an AttoLang expression compiled and executed in Wasm.
// Distinct, independent guest programs detect both valid and invalid cases.
const cases = [
  ['ethernet-bounds', 'valid_ethernet_frame(1514, 1500) and !valid_ethernet_frame(1515, 1500)'],
  ['mac-bounds', 'valid_mac(281474976710655) and !valid_mac(-1)'],
  ['nic-ring-full', 'valid_descriptor_ring(8, 8) and !can_receive(60, 1500, 8, 8, true)'],
  ['nic-ring-wrap', 'next_ring_slot(7, 8) == 0 and available_descriptors(4, 8) == 4'],
  ['nic-link-down', '!can_transmit(60, 1500, 0, 8, false)'],
  ['guest-last-byte', 'valid_guest_range(4095, 1, 4096) and !valid_guest_range(4095, 2, 4096)'],
  ['guest-no-zero-length', '!valid_guest_range(0, 0, 4096) and !valid_guest_range(-1, 1, 4096)'],
  ['virtqueue-wrap', 'split_ring_pending(0, 65535) == 1 and split_ring_can_consume(0, 65535, 8)'],
  ['virtqueue-overrun', '!split_ring_can_consume(600, 0, 256) and !supported_split_ring_size(3)'],
  ['descriptor-rx-write', 'valid_flat_descriptor(0, 1024, 4096, 2, 2) and !valid_flat_descriptor(0, 1024, 4096, 0, 2)'],
  ['descriptor-tx-read', 'valid_flat_descriptor(0, 1024, 4096, 0, 0) and !valid_flat_descriptor(0, 1024, 4096, 2, 0)'],
  ['batch-generation', 'can_start_fast_path_batch(9, 9, 1, 1, 1, 0, 8, true, false) and !can_start_fast_path_batch(9, 9, 1, 2, 1, 0, 8, true, false)'],
  ['tagged-ethernet', 'valid_virtual_ethernet_frame(1518, 1500, 100) and !valid_virtual_ethernet_frame(1519, 1500, 100)'],
  ['virtio-header-no-offload', 'virtio_net_header_supported(10, 0, 0) and !virtio_net_header_supported(10, 1, 0)'],
  ['rx-descriptor-capacity', 'nic_can_consume_rx(9,9,1,1,1,0,8,0,1536,4096,2,10,0,0,1514,1500,0,true,true,false)'],
  ['rx-reject-readonly-descriptor', '!nic_can_consume_rx(9,9,1,1,1,0,8,0,1536,4096,0,10,0,0,1514,1500,0,true,true,false)'],
  ['switch-handoff-admission', 'nic_can_handoff_switch(1514,1500,0,1,4,true) and !nic_can_handoff_switch(1514,1500,0,4,4,true)'],

  ['queue-u16-counter-wrap', 'next_u16_queue_index(65535) == 0 and next_u16_queue_index(-1) == -1'],
  ['queue-full-vs-empty', 'queue_can_offer(0,0,8) and !queue_can_claim(0,0,8) and !queue_can_offer(8,0,8)'],
  ['queue-descriptor-lifecycle', 'next_descriptor_state(0,1,4,4) == 1 and next_descriptor_state(1,2,4,4) == 2 and next_descriptor_state(2,3,4,4) == 3 and next_descriptor_state(3,4,4,4) == 0'],
  ['queue-stale-descriptor-epoch', 'next_descriptor_state(1,2,4,3) == -1 and next_descriptor_state(2,2,4,4) == -1'],
  ['queue-rx-claim', 'queue_can_claim_flat(9,9,1,1,1,0,8,0,1,0,1536,4096,2,true,true,false)'],
  ['queue-tx-claim', 'queue_can_claim_flat(9,9,1,1,1,0,8,0,1,0,1536,4096,0,false,true,false)'],
  ['queue-reject-forged-direction', '!queue_can_claim_flat(9,9,1,1,1,0,8,0,1,0,1536,4096,0,true,true,false) and !queue_can_claim_flat(9,9,1,2,1,0,8,0,1,0,1536,4096,2,true,true,false)'],
  ['queue-drain-during-quiesce', 'queue_next_used_after_complete(9,9,1,1,1,0,8,2,true,true) == 1 and queue_next_used_after_complete(9,9,1,2,1,0,8,2,true,true) == -1'],
];
const work = mkdtempSync(join(tmpdir(), 'atto-vrtl-nic-guest-'));
const receipt = [];
const run = (source, output) => {
  const r = spawnSync(compiler, [source, '-o', output], {
    encoding: 'utf8', timeout: 30_000, maxBuffer: 1 << 20,
  });
  assert.equal(r.error, undefined, 'attoc execution failed');
  assert.notEqual(r.status, null, 'attoc crashed or timed out');
  return r;
};
try {
  for (const [name, condition] of cases) {
    const attoSource = fullSource +
      'pub fnc main(): int { if ' + condition +
      ' then return 42; else return 0; fi }\n';
    const input = join(work, name + '.atto');
    const out1 = join(work, name + '.wasm');
    const out2 = join(work, name + '-repeat.wasm');
    writeFileSync(input, attoSource);
    const result = run(input, out1);
    assert.equal(result.status, 0, name + ': compiler rejected real Atto device source: ' + result.stderr);
    assert.ok(existsSync(out1), name + ': compiled guest artifact missing');
    const bytes = readFileSync(out1);
    assert.ok(WebAssembly.validate(bytes), name + ': invalid generated Wasm');
    const module = new WebAssembly.Module(bytes);
    assert.deepEqual(WebAssembly.Module.imports(module), [], name + ': pure-device guest cannot use host imports');
    const guest = new WebAssembly.Instance(module, {});
    assert.equal(guest.exports.main(), 42n, name + ': incorrect executed Atto guest result');
    assert.equal(guest.exports.panic_code.value, 0, name + ': unexpected guest panic');
    const second = run(input, out2);
    assert.equal(second.status, 0, name + ': repeat compile failed: ' + second.stderr);
    assert.deepEqual(readFileSync(out2), bytes, name + ': nondeterministic compiler output');
    receipt.push({ name, status: 'compiled_executed', main_i64: 42, wasm_sha256: sha256(bytes) });
    console.log('ATTO_GUEST_PASS', name, sha256(bytes));
  }
  // Negative compiler proof: unresolved host KVM symbol must not gain a
  // linker stub, implicit host import or output Wasm.
  const rejectSource = join(work, 'negative-unsupported-kvm.atto');
  writeFileSync(rejectSource, fullSource +
    'pub fnc main(): int { return host_kvm_ioctl(42); }\n');
  let diagnostic = null;
  for (let n = 0; n < 2; n++) {
    const out = join(work, 'negative-' + n + '.wasm');
    const r = run(rejectSource, out);
    assert.equal(r.status, 1, 'unverified KVM call must be rejected');
    assert.ok(!existsSync(out), 'rejected source emitted Wasm');
    const found = r.stderr.match(/E_[A-Z0-9_]+/);
    assert.ok(found, 'missing typed compiler diagnostic: ' + r.stderr);
    if (diagnostic !== null) assert.equal(found[0], diagnostic);
    diagnostic = found[0];
  }
  const proof = {
    schema:'atto.virtual.hardware.nic.guest-execution.v1',
    source_repo:manifest.source_repo, source_commit:manifest.source_commit,
    source_git_blobs:manifest.files.map(f => ({path:f.path,git_blob_sha1:f.git_blob_sha1})),
    compiler_sha256:sha256(readFileSync(compiler)),
    runtime:'Node WebAssembly engine',
    native_aot:false, kvm_executed:false, control_actor_executed:false,
    compiled_exact_function_ranges:compiledFunctions,
    excluded_functions_pending_integer_division:excluded,
    full_original_module_executed:false,
    tests:receipt, rejected_kvm_symbol:{status:'rejected_no_artifact',diagnostic},
  };
  console.log('ATTO_VRTL_NIC_GUEST_RECEIPT\n' + JSON.stringify(proof, null, 2));
} finally {
  rmSync(work, {recursive:true, force:true});
}

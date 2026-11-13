// Actual Rust/Wasm Atto compiler -> generated guest Wasm -> current
// playground's real tenant-local actor scheduler. No JS source interpreter.
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { resolve, join } from 'node:path';
import { pathToFileURL } from 'node:url';

const site = process.env.ATTO_PLAY_SITE_ROOT;
if (!site) throw new Error('Set ATTO_PLAY_SITE_ROOT to an exact-reviewed playground checkout');
const moduleFromSite = p => import(pathToFileURL(join(resolve(site), 'play/src', p)).href);
const [{createCompiler}, {actorNames, guestImports, pipeGuestAbi}, {createActorHost}] = await Promise.all([
  moduleFromSite('compiler.js'), moduleFromSite('guest.js'), moduleFromSite('actor-host.js')
]);
const wasmPath = process.argv[2] ?? 'target/wasm32-unknown-unknown/release/atto_compiler.wasm';
const compiler = await createCompiler(await readFile(wasmPath));

const stages = `
define actor Parse as
  receive(ActorMail<int> mail): void {
    emit(mail.value + 1);
  }
end
define actor Validate as
  receive(ActorMail<int> mail): void {
    if mail.value > 0 then emit(mail.value); fi
  }
end
define actor Sink as
  receive(ActorMail<int> mail): void {
    stdio.println(mail.value);
  }
end
`;

function compile(source) {
  const r = compiler.compile(source);
  assert.equal(r.ok, true, JSON.stringify(r.diagnostics));
  assert.equal(WebAssembly.validate(r.wasm), true);
  return new WebAssembly.Module(r.wasm);
}

async function execute(module) {
  const out = {text: '', truncated: false};
  const host = createActorHost({module, names: actorNames(module), out});
  const {imports, bind} = guestImports(module, out, host);
  const main = await WebAssembly.instantiate(module, imports);
  bind(main);
  assert.equal(main.exports.main(), 0n);
  const fault = await host.drain();
  return {out, host, fault};
}

// Two real isolated pipelines with an interleaved main-thread submission.
const program = stages + `
pub fnc main(): int {
  val one = spawn Parse() pipe spawn Validate() pipe spawn Sink();
  val two = spawn Parse() | spawn Validate() | spawn Sink();
  one.send(41);
  two.send(100);
  one.send(42);
  return 0;
}
`;
const module = compile(program);
assert.equal(pipeGuestAbi(module), true, 'compiler must opt into the actual pipeline-v1 marker');
assert.deepEqual([...new Uint8Array(WebAssembly.Module.customSections(module, 'atto.pipeline.v1')[0])], [1]);
assert.equal(WebAssembly.Module.customSections(module, 'atto.actor.abi').length, 0,
  'v1 pipeline extension cannot impersonate resumable ABI v3');
assert.deepEqual(actorNames(module), ['Parse','Validate','Sink']);
const guestImportsActual = WebAssembly.Module.imports(module).filter(i => i.module === 'atto').map(i => i.name);
for (const required of ['actor_spawn','actor_send','actor_link','actor_emit']) {
  assert.ok(guestImportsActual.includes(required), required);
}
const run = await execute(module);
assert.equal(run.fault, null);
assert.equal(run.out.text, '42\n101\n43\n', 'each Wasm actor must forward in deterministic mailbox order');
assert.equal(run.out.truncated, false);
assert.equal(run.host.count, 6);
assert.equal(run.host.memoryBytes, 6 * 65536, 'every actor has its own 64 KiB Wasm32 instance memory');
run.host.close();
assert.equal(run.host.memoryBytes, 0);
console.log('PASS: real compiler emitted two private-memory actor pipelines and forwarded 42, 101, 43');

// Source type mismatch must never publish a guest, even though it has valid syntax.
const mismatch = compiler.compile(stages.replace('ActorMail<int> mail): void {\n    if',
  'ActorMail<bool> mail): void {\n    if') + `
pub fnc main(): int {
  val p = spawn Parse() pipe spawn Validate() pipe spawn Sink();
  p.send(41);
  return 0;
}`);
assert.equal(mismatch.ok, false);
assert.match(mismatch.diagnostics[0].message, /^E_TYPE:/);

// A source-origin emit followed by a Wasm panic MUST NOT be committed downstream.
const crashing = compile(stages.replace('emit(mail.value + 1);',
  'emit(mail.value + 1); stdio.println(mail.value * 9223372036854775807);') + `
pub fnc main(): int {
  val p = spawn Parse() pipe spawn Validate() pipe spawn Sink();
  p.send(7);
  return 0;
}`);
const faulted = await execute(crashing);
assert.equal(faulted.fault, 'E_PANIC: integer overflow in actor Parse#0');
assert.equal(faulted.out.text, '', 'a failed actor turn cannot partially commit emitted mail');
assert.equal(faulted.host.count, 3);
faulted.host.close();
console.log('PASS: typed pipeline mismatch rejected and emitted mail rolled back on actor panic');

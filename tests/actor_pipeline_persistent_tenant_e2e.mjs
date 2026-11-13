// Compiler-produced actor pipeline running in persistent real Node Worker threads.
// No mocked Worker, no JS actor source interpreter, no shared tenant memory.
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { resolve, join } from 'node:path';
import { pathToFileURL } from 'node:url';

const site = process.env.ATTO_PLAY_SITE_ROOT;
if (!site) throw new Error('ATTO_PLAY_SITE_ROOT must point to exact reviewed site checkout');
const moduleFromSite = p => import(pathToFileURL(join(resolve(site), 'play', p)).href);
const [{createCompiler}, {createTenantRegistry}] = await Promise.all([
  moduleFromSite('src/compiler.js'),
  moduleFromSite('node/tenant-session.mjs')
]);
const compiler = await createCompiler(await readFile(
  process.argv[2] ?? 'target/wasm32-unknown-unknown/release/atto_compiler.wasm'));
const program = `
define actor Parse as
  receive(ActorMail<int> mail): void {
    emit(mail.value + 1);
    if mail.value eq 7 then
      stdio.println(mail.value * 9223372036854775807);
    fi
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
pub fnc main(): int {
  val first = spawn Parse() pipe spawn Validate() pipe spawn Sink();
  val second = spawn Parse() pipe spawn Validate() pipe spawn Sink();
  return 0;
}`;
const result = compiler.compile(program);
assert.equal(result.ok,true,JSON.stringify(result.diagnostics));
const guest = new Uint8Array(result.wasm);
assert.equal(WebAssembly.validate(guest),true);
const registry = createTenantRegistry({maxTenants:2,timeoutMs:10_000});
let alpha;
try{
  const sa=registry.open('alpha');
  const sb=registry.open('beta');
  assert.equal(registry.size,2);
  alpha=await sa.start(guest);
  const beta=await sb.start(guest);
  assert.equal(alpha.actorHandles.length,6);
  assert.equal(beta.actorHandles.length,6);
  assert.equal(alpha.value,'0');
  assert.equal(beta.value,'0');

  // An opaque ActorRef from another Worker must never become a valid handle.
  await assert.rejects(sb.send(alpha.actorHandles[0],41n),/invalid or cross-tenant ActorRef/);
  const good=await sb.send(beta.actorHandles[0],41n);
  assert.equal(good.ok,true);
  assert.equal(good.stdout,'42\n');

  // Actor 0 emits then panics. No mail may reach its successor.
  const bad=await sa.send(alpha.actorHandles[0],7n);
  assert.equal(bad.ok,false);
  assert.equal(bad.actorFault,true);
  assert.match(bad.message,/E_PANIC: integer overflow in actor Parse#0/);
  assert.equal(bad.stdout,'', 'failed receive transaction rolls back its emitted mail');
  assert.equal(sa.closed,false,'actor panic does not destroy surviving sibling actors');
  const denied=await sa.send(alpha.actorHandles[0],41n);
  assert.equal(denied.ok,false);
  assert.match(denied.message,/E_ACTOR_DEAD/);

  // A separate chain in that SAME tenant continues serving after panic.
  const survivor=await sa.send(alpha.actorHandles[3],41n);
  assert.equal(survivor.ok,true);
  assert.equal(survivor.stdout,'42\n');
  // Other tenant remains usable throughout the first tenant's actor fault.
  const other=await sb.send(beta.actorHandles[3],100n);
  assert.equal(other.ok,true);
  assert.equal(other.stdout,'101\n');

  const retired=await registry.close('alpha');
  assert.equal(retired,true);
  assert.equal(sa.retired,true,'registry must await physical Worker termination');
  assert.equal(registry.size,1);
  const replacement=registry.open('alpha');
  const restarted=await replacement.start(guest);
  await assert.rejects(replacement.send(alpha.actorHandles[3],41n),
    /invalid or cross-tenant ActorRef/,
    'reused tenant ID must not make stale ActorRefs valid');
  const next=await replacement.send(restarted.actorHandles[3],10n);
  assert.equal(next.ok,true);
  assert.equal(next.stdout,'11\n');
  console.log('PASS TenantVM: actor panic rolls back emit and healthy sibling/other tenant continue');
  console.log('PASS TenantVM: cross-tenant/stale ActorRefs rejected and Worker retirement fenced');
} finally {
  await registry.closeAll();
  assert.equal(registry.size,0);
}

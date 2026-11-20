import test from 'node:test';
import assert from 'node:assert/strict';
import {StdioTextHostV1} from '../runtime/browser/stdio_text_host_v1.mjs';
import {createStdioTextGuestV1,STDIO_TEXT_ERRORS as E}
  from '../runtime/browser/stdio_text_guest_v1.mjs';

const encoder=new TextEncoder(),decoder=new TextDecoder();
async function* input(...lines) {for(const line of lines)yield line instanceof Uint8Array?line:encoder.encode(line)}
function fixture({stdin=input('hello\n'),grant={}}={}) {
  const stdout=[],stderr=[],memory=new WebAssembly.Memory({initial:2,maximum:2});
  const host=new StdioTextHostV1({tenantId:'trusted-tenant',generation:42,
    stdin,stdout:async bytes=>stdout.push(decoder.decode(bytes)),
    stderr:async bytes=>stderr.push(decoder.decode(bytes))});
  const bridge=createStdioTextGuestV1({getMemory:()=>memory,host,
    grant:{tenantId:'trusted-tenant',generation:42,
      allowStdin:true,allowStdout:true,allowStderr:true,...grant}});
  const view=new DataView(memory.buffer);
  return {host,bridge,memory,view,stdout,stderr};
}

test('read import commits UTF-8 bytes and checked Some, None as distinct tags',async()=>{
  const f=fixture({stdin:input('é\n')});
  assert.equal(await f.bridge.imports.stdio_text_read_v1(0,64,128,16),E.Ok);
  assert.equal(f.view.getInt32(16,true),0);
  assert.equal(f.view.getInt32(24,true),2);
  assert.equal(decoder.decode(new Uint8Array(f.memory.buffer,64,2)),'é');
  assert.equal(await f.bridge.imports.stdio_text_read_v1(0,64,128,16),E.Ok);
  assert.equal(f.view.getInt32(16,true),1,'EOF is tagged None, not empty string');
  assert.equal(f.view.getInt32(24,true),0);
});

test('empty LF text is Some("") and malformed UTF-8 is typed Err',async()=>{
  const empty=fixture({stdin:input('\n')});
  assert.equal(await empty.bridge.imports.stdio_text_read_v1(0,64,128,16),E.Ok);
  assert.equal(empty.view.getInt32(16,true),0);
  assert.equal(empty.view.getInt32(24,true),0);
  const invalid=fixture({stdin:input(new Uint8Array([0xc3,0x28,0x0a]))});
  assert.equal(await invalid.bridge.imports.stdio_text_read_v1(0,64,128,16),E.Ok);
  assert.equal(invalid.view.getInt32(16,true),2);
  assert.equal(invalid.view.getInt32(20,true),E.MalformedUtf8);
});

test('checked write import has deterministic output identity and no implicit newline',async()=>{
  const f=fixture();
  const data=encoder.encode('β\0echo');
  new Uint8Array(f.memory.buffer,64,data.length).set(data);
  assert.equal(await f.bridge.imports.stdio_text_write_v1(1,64,data.length),E.Ok);
  assert.equal(await f.bridge.imports.stdio_text_write_v1(2,64,data.length),E.Ok);
  assert.deepEqual(f.stdout,['β\0echo']);
  assert.deepEqual(f.stderr,['β\0echo']);
});

test('denied port grants are checked before any host read or write',async()=>{
  const denied=fixture({grant:{allowStdin:false,allowStdout:false,allowStderr:false}});
  assert.equal(await denied.bridge.imports.stdio_text_read_v1(0,64,128,16),E.Denied);
  assert.equal(await denied.bridge.imports.stdio_text_write_v1(1,64,0),E.Denied);
  assert.equal(await denied.bridge.imports.stdio_text_write_v1(2,64,0),E.Denied);
  assert.deepEqual(denied.stdout,[]);
  assert.deepEqual(denied.stderr,[]);
  assert.equal(await denied.bridge.imports.stdio_text_read_v1(1,64,128,16),E.Denied);
});

test('guest-owned memory boundaries, UTF-8 validation and quotas fail closed',async()=>{
  const f=fixture();
  assert.equal(await f.bridge.imports.stdio_text_read_v1(0,131071,128,16),E.InvalidPointer);
  assert.equal(await f.bridge.imports.stdio_text_read_v1(0,64,0,16),E.InvalidCapacity);
  assert.equal(await f.bridge.imports.stdio_text_read_v1(0,64,128,15),E.InvalidPointer);
  assert.equal(await f.bridge.imports.stdio_text_write_v1(1,131071,16),E.InvalidPointer);
  assert.equal(await f.bridge.imports.stdio_text_write_v1(1,64,65537),E.Quota);
  new Uint8Array(f.memory.buffer,64,2).set([0xc3,0x28]);
  assert.equal(await f.bridge.imports.stdio_text_write_v1(1,64,2),E.MalformedUtf8);
});

test('overlapping result and input buffers are rejected before consuming stdin',async()=>{
  const f=fixture({stdin:input('not-lost\n')});
  // A result slot overlapping the guest-owned byte destination would
  // overwrite the read payload when the JSPI import commits its tag.
  assert.equal(await f.bridge.imports.stdio_text_read_v1(0,64,128,64),E.InvalidPointer);
  assert.equal(await f.bridge.imports.stdio_text_read_v1(0,64,128,176),E.InvalidPointer);
  // Rejected reads must not consume the injected line.
  assert.equal(await f.bridge.imports.stdio_text_read_v1(0,64,128,16),E.Ok);
  const length=f.view.getInt32(24,true);
  assert.equal(decoder.decode(new Uint8Array(f.memory.buffer,64,length)),'not-lost');
});

test('grant identity and generation are mandatory, never chosen by the guest',()=>{
  const f=fixture();
  assert.throws(()=>createStdioTextGuestV1({
    getMemory:()=>f.memory,host:f.host,grant:{tenantId:'wrong',generation:42,
      allowStdin:true,allowStdout:true,allowStderr:true}}),/generation or permission/);
  assert.throws(()=>createStdioTextGuestV1({
    getMemory:()=>f.memory,host:f.host,grant:{tenantId:'trusted-tenant',generation:43,
      allowStdin:true,allowStdout:true,allowStderr:true}}),/generation or permission/);
  assert.throws(()=>createStdioTextGuestV1({
    getMemory:()=>f.memory,host:f.host,grant:{tenantId:'trusted-tenant',generation:42,
      allowStdin:true,allowStdout:true}}),/generation or permission/);
});

test('revocation cancels suspended stdin without committing late guest memory',async()=>{
  let send;
  const stdin=(async function*(){yield await new Promise(resolve=>{send=resolve})})();
  const f=fixture({stdin});
  // Even if a malicious guest leaves a writable result region in memory,
  // revocation must not allow late host data to appear in that region.
  new Uint8Array(f.memory.buffer,64,128).fill(0xA5);
  new Uint8Array(f.memory.buffer,16,16).fill(0x5A);
  const pending=f.bridge.imports.stdio_text_read_v1(0,64,128,16);
  f.bridge.revoke();
  assert.equal(await pending,E.Revoked);
  assert.deepEqual([...new Uint8Array(f.memory.buffer,64,128)],Array(128).fill(0xA5));
  assert.deepEqual([...new Uint8Array(f.memory.buffer,16,16)],Array(16).fill(0x5A));
  assert.equal(await f.bridge.imports.stdio_text_read_v1(0,64,128,16),E.Revoked);
  assert.equal(await f.bridge.imports.stdio_text_write_v1(1,64,0),E.Revoked);
  // The injected iterator may finish later; a revoked bridge has no path
  // back to this tenant's guest memory.
  if (send) send(encoder.encode('post-revoke\n'));
  await Promise.resolve();
  assert.deepEqual([...new Uint8Array(f.memory.buffer,64,128)],Array(128).fill(0xA5));
});

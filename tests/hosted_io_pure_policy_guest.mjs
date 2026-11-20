// Compile canonical .atto pure HTTP/FS policy via native Rust attoc and run guest Wasm.
// This does NOT implement actual HTTP get, file I/O or package linking.
import assert from 'node:assert/strict';
import {spawnSync} from 'node:child_process';
import {mkdtempSync,readFileSync,writeFileSync,rmSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {join} from 'node:path';
import {createHash} from 'node:crypto';

const compiler=process.env.ATTOC || 'target/release/attoc';
const dir=mkdtempSync(join(tmpdir(),'atto-io-policy-guest-'));
const sources=[
 ['../stdlib/std/http_response_policy.atto','22757dafe89bc9601d04c8726a09ddf676206c7e'],
 ['../stdlib/std/fs_text_policy.atto','0cd53216e91ef8de1a764ba5b80ce85c5796c1f2'],
];
const blobSha=data=>createHash('sha1').update('blob '+data.byteLength+'\0').update(data).digest('hex');
const parts=sources.map(([path,sha])=>{
 const bytes=readFileSync(new URL(path,import.meta.url));
 assert.equal(blobSha(bytes),sha,'canonical source SHA drifted: '+path);
 return bytes.toString('utf8');
});
const checks=[
'pub fnc main(): int {',
'  if http_status_valid(100) == false then return 1; fi',
'  if http_status_valid(600) then return 2; fi',
'  if http_status_success(200) == false then return 3; fi',
'  if http_status_success(404) then return 4; fi',
'  if http_status_client_error(404) == false then return 5; fi',
'  if http_status_server_error(503) == false then return 6; fi',
'  if http_get_status_may_have_body(404) == false then return 7; fi',
'  if http_get_status_may_have_body(204) then return 8; fi',
'  if http_get_status_may_have_body(205) then return 9; fi',
'  if http_get_status_may_have_body(304) then return 10; fi',
'  if fs_text_size_admissible(10, 10, 10) == false then return 11; fi',
'  if fs_text_size_admissible(11, 10, 100) then return 12; fi',
'  if fs_text_range_admissible(9, 2, 10) then return 13; fi',
'  if fs_text_range_admissible(9, 1, 10) == false then return 14; fi',
'  if fs_text_replacement_admissible(4, 7, 5, 8) == false then return 15; fi',
'  if fs_text_replacement_admissible(4, 8, 5, 8) then return 16; fi',
'  return 42;',
'}'
];
const program=parts.join('\n')+'\n'+checks.join('\n')+'\n';
try{
 const filename=join(dir,'hosted-io-policy.atto');
 writeFileSync(filename,program);
 const compiled=spawnSync(compiler,[filename],{timeout:30000,maxBuffer:4*1024*1024});
 assert.equal(compiled.error,undefined,compiled.error?.message);
 assert.equal(compiled.status,0,compiled.stderr?.toString()||'attoc did not return success');
 assert.ok(WebAssembly.validate(compiled.stdout),'native compiler emitted invalid Wasm');
 const module=new WebAssembly.Module(compiled.stdout);
 assert.deepEqual(WebAssembly.Module.imports(module),[],
   'scalar policy guest must not import ambient HTTP/FS host I/O');
 const instance=new WebAssembly.Instance(module,{});
 assert.equal(instance.exports.main(),42n,'compiled Atto policy returned the wrong value');
 console.log('PASS compiled native Atto HTTP/FS scalar policy guest: exit 42, zero host imports');
} finally {
 rmSync(dir,{recursive:true,force:true});
}

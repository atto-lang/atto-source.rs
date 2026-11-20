// Compile canonical .atto pure HTTP/FS policy via native Rust attoc and run guest Wasm.
// This tests the actual sealed package import/linker path, NOT HTTP or filesystem I/O.
import assert from 'node:assert/strict';
import {spawnSync} from 'node:child_process';
import {mkdtempSync,readFileSync,writeFileSync,mkdirSync,rmSync} from 'node:fs';
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
for(const [path,sha] of sources){
 const bytes=readFileSync(new URL(path,import.meta.url));
 assert.equal(blobSha(bytes),sha,'canonical source SHA drifted: '+path);
}
const checks=[
'pub fnc main(): int {',
'  if http.http_status_valid(100) == false then return 1; fi',
'  if http.http_status_valid(600) then return 2; fi',
'  if http.http_status_success(200) == false then return 3; fi',
'  if http.http_status_success(404) then return 4; fi',
'  if http.http_status_client_error(404) == false then return 5; fi',
'  if http.http_status_server_error(503) == false then return 6; fi',
'  if http.http_get_status_may_have_body(404) == false then return 7; fi',
'  if http.http_get_status_may_have_body(204) then return 8; fi',
'  if http.http_get_status_may_have_body(205) then return 9; fi',
'  if http.http_get_status_may_have_body(304) then return 10; fi',
'  if fs.fs_text_size_admissible(10, 10, 10) == false then return 11; fi',
'  if fs.fs_text_size_admissible(11, 10, 100) then return 12; fi',
'  if fs.fs_text_range_admissible(9, 2, 10) then return 13; fi',
'  if fs.fs_text_range_admissible(9, 1, 10) == false then return 14; fi',
'  if fs.fs_text_replacement_admissible(4, 7, 5, 8) == false then return 15; fi',
'  if fs.fs_text_replacement_admissible(4, 8, 5, 8) then return 16; fi',
'  return 42;',
'}'
];
const program=[
 'import * as http from pkg "std.http";',
 'import * as fs from pkg "std.fs";',
 ...checks
].join('\n')+'\n';
try{
 const filename=join(dir,'hosted-io-policy.atto');
 const compile=source=>{
   writeFileSync(filename,source);
   return spawnSync(compiler,[filename],{timeout:30000,maxBuffer:4*1024*1024});
 };
 const compiled=compile(program);
 assert.equal(compiled.error,undefined,compiled.error?.message);
 assert.equal(compiled.status,0,compiled.stderr?.toString()||'attoc did not return success');
 assert.ok(WebAssembly.validate(compiled.stdout),'native compiler emitted invalid Wasm');
 const module=new WebAssembly.Module(compiled.stdout);
 assert.deepEqual(WebAssembly.Module.imports(module),[],
   'scalar policy guest must not import ambient HTTP/FS host I/O');
 const instance=new WebAssembly.Instance(module,{});
 assert.equal(instance.exports.main(),42n,'compiled Atto policy returned the wrong value');
 // Import source must come from pinned compiler-embedded bytes, not a
 // matching-looking untrusted local package or source directory.
 const forged=join(dir,'stdlib','std');
 mkdirSync(forged,{recursive:true});
 writeFileSync(join(forged,'http_response_policy.atto'),
   'pub fnc http_status_valid(int s): bool { return false; }\n');
 const repeated=compile(program);
 assert.equal(repeated.status,0,repeated.stderr?.toString());
 assert.deepEqual(repeated.stdout,compiled.stdout,'compiler must ignore forged stdlib source');
 for(const [label,source,code] of [
   ['unlisted package','import * as x from pkg "other.http"; pub fnc main(): int { return 42; }','E_UNSUPPORTED'],
   ['unimplemented fetch','import * as http from pkg "std.http"; pub fnc main(): int { return http.get(1); }','E_UNBOUND'],
   ['unimplemented filesystem','import * as fs from pkg "std.fs"; pub fnc main(): int { return fs.read_text(1); }','E_UNBOUND'],
   ['duplicate binding','import * as x from pkg "std.http"; import * as x from pkg "std.fs"; pub fnc main(): int {return 0;}','E_DUPLICATE'],
   ['forged reserved name','import * as http from pkg "std.http"; pub fnc __atto_file_import_0(): int {return 0;} pub fnc main(): int {return 42;}','E_RESERVED'],
 ]) {
   const result=compile(source);
   assert.equal(result.status,1,label+': '+result.stderr?.toString());
   assert.equal(result.stdout.length,0,label+' emitted a Wasm artifact');
   assert.ok(result.stderr?.toString().includes(code),label+': '+result.stderr?.toString());
 }

 // The originally requested async GET -> file write/read guest program is
 // not lowered yet. Verify the production compiler emits no misleading Wasm.
 const realIo=compile([
   'import * as http from pkg "std.http";',
   'import * as fs from pkg "std.fs";',
   'pub susp fnc main(): int {',
   '  val response = await http.get("http://127.0.0.1:8080/data");',
   '  val body = await response.text();',
   '  await fs.write_text("./response.txt", body);',
   '  val saved = await fs.read_text("./response.txt");',
   '  return saved.length;',
   '}'
 ].join('\n'));
 assert.equal(realIo.status,1,
   'unimplemented async HTTP/FS must not produce a success exit code');
 assert.equal(realIo.stdout.length,0,
   'unimplemented async HTTP/FS unexpectedly produced executable Wasm');
 console.log('PASS compiled Atto sealed std HTTP/FS pure-policy imports: exit 42, zero host imports, five negative cases plus rejected async HTTP/FS');
} finally {
 rmSync(dir,{recursive:true,force:true});
}

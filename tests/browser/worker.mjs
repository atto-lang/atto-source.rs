// This worker loads the *real* Rust-compiled compiler wasm; no JS evaluator.
import {createStateRuntime} from './state-runtime.mjs';
// Java Double.toString (JDK 19+ shortest round-trip); see tests/java-double.mjs
function javaDouble(x){
  if(Number.isNaN(x))return 'NaN';
  if(x===Infinity)return 'Infinity';
  if(x===-Infinity)return '-Infinity';
  if(x===0)return Object.is(x,-0)?'-0.0':'0.0';
  const a=Math.abs(x);
  if(a>=1e-3&&a<1e7){const s=String(x);return s.includes('.')?s:s+'.0';}
  const [m,e]=x.toExponential().split('e');
  return (m.includes('.')?m:m+'.0')+'E'+e.replace('+','');
}

// Atto browser guest ABI v1: verify binary Wasm imported function signatures.
// Module.imports() exposes only names/kinds; it does NOT expose signatures.
const ABI_SIGNATURES = Object.freeze({
  print_i64: [[0x7e], []],
  print_bool: [[0x7f], []],
  print_str: [[0x7f, 0x7f], []],
  print_newline: [[], []],
  print_f64: [[0x7c], []],
  state_register: [[0x7f], []],
  state_enter: [[0x7f, 0x7f], [0x7e]],
  state_leave: [[0x7e], []],
});
function abiReject(reason) { throw new Error('E_ABI: ' + reason); }
const abiDecoder = new TextDecoder('utf-8', {fatal: true});

export function validateGuestImportABI(bytes, module = null) {
  if (!(bytes instanceof Uint8Array) || bytes.byteLength < 8 || bytes.byteLength > 64 * 1024 * 1024)
    abiReject('invalid guest byte length');
  if (!bytes.subarray(0, 8).every((v, i) => v === [0, 97, 115, 109, 1, 0, 0, 0][i]))
    abiReject('invalid guest wasm header/version');
  let pos = 8, types = [], imported = [], seenTypes = false, seenImports = false;
  const get = (end) => { if (pos >= end) abiReject('truncated guest ABI section'); return bytes[pos++]; };
  const u32 = (end) => {
    let n = 0;
    for (let i = 0; i < 5; i++) {
      const b = get(end);
      if (i === 4 && (b & 0xf0)) abiReject('overflowed LEB32');
      n += (b & 0x7f) * (2 ** (7 * i));
      if (!(b & 0x80)) {
        if (i && (b & 0x7f) === 0) abiReject('noncanonical LEB32');
        return n;
      }
    }
    abiReject('unterminated LEB32');
  };
  const name = (end) => {
    const n = u32(end);
    if (n > 128 || n > end - pos) abiReject('invalid guest import name length');
    let result;
    try { result = abiDecoder.decode(bytes.subarray(pos, pos + n)); }
    catch { abiReject('invalid UTF-8 import name'); }
    pos += n;
    return result;
  };
  const valtypes = (end, max) => {
    const count = u32(end);
    if (count > max) abiReject('oversize ABI signature');
    const v = [];
    for (let i = 0; i < count; i++) {
      const kind = get(end);
      // Only reviewed numeric value types in browser guest function signatures.
      if (![0x7f, 0x7e, 0x7d, 0x7c].includes(kind)) abiReject('unknown guest ABI value type');
      v.push(kind);
    }
    return v;
  };
  while (pos < bytes.length) {
    const id = get(bytes.length);
    const len = u32(bytes.length);
    if (len > bytes.length - pos) abiReject('truncated wasm section');
    const end = pos + len;
    if (id === 1) {
      if (seenTypes || seenImports) abiReject('duplicate or reordered type section');
      seenTypes = true;
      const count = u32(end);
      if (count > 65536) abiReject('too many guest ABI types');
      for (let i = 0; i < count; i++) {
        if (get(end) !== 0x60) abiReject('unsupported guest ABI type form');
        types.push([valtypes(end, 256), valtypes(end, 16)]);
      }
      if (pos !== end) abiReject('malformed type section');
    } else if (id === 2) {
      if (seenImports) abiReject('duplicate guest import section');
      seenImports = true;
      const count = u32(end);
      if (count > 128) abiReject('too many guest imports');
      const used = new Set();
      for (let i = 0; i < count; i++) {
        const mod = name(end), symbol = name(end), kind = get(end);
        if (mod !== 'atto' || !Object.hasOwn(ABI_SIGNATURES, symbol))
          abiReject('guest import not granted: ' + mod + '.' + symbol);
        if (kind !== 0) abiReject('foreign non-function import: ' + symbol);
        if (used.has(symbol)) abiReject('duplicate guest import: ' + symbol);
        used.add(symbol);
        const index = u32(end);
        const actual = types[index], expected = ABI_SIGNATURES[symbol];
        if (!actual || !actual.every((group, k) =>
          group.length === expected[k].length && group.every((x, j) => x === expected[k][j])))
          abiReject('guest import signature mismatch: ' + symbol);
        imported.push({module: mod, name: symbol, kind: 'function'});
      }
      if (pos !== end) abiReject('malformed guest import section');
    }
    pos = end;
  }
  if (module) {
    const reflected = WebAssembly.Module.imports(module);
    if (imported.length !== reflected.length || imported.some((entry, i) =>
      entry.module !== reflected[i].module || entry.name !== reflected[i].name ||
      entry.kind !== reflected[i].kind)) abiReject('binary/reflection import mismatch');
  }
  return imported;
}

self.onmessage=async ({data})=>{
  try{
    const bytes=await (await fetch('/atto_compiler.wasm')).arrayBuffer();
    if(!WebAssembly.validate(bytes))throw new Error('invalid compiler wasm');
    const compiler=new WebAssembly.Module(bytes);
    if(WebAssembly.Module.imports(compiler).length)throw new Error('unexpected compiler imports');
    const api=new WebAssembly.Instance(compiler,{}).exports;
    const encoded=new TextEncoder().encode(data.source);
    if(encoded.length>api.input_capacity())throw new Error('source too big');
    new Uint8Array(api.memory.buffer,api.input_ptr(),encoded.length).set(encoded);
    if(api.compile(encoded.length)!==0){
      if(api.result_length()!==0)throw new Error('compiler emitted on rejection');
      const mem=new Uint8Array(api.memory.buffer);const start=api.error_ptr();let end=start;
      while(end<mem.length&&mem[end]&&end-start<256)end++;
      self.postMessage({ok:false,error:new TextDecoder().decode(mem.subarray(start,end))});
      return;
    }
    const out=new Uint8Array(api.memory.buffer,api.result_ptr(),api.result_length()).slice();
    if(!WebAssembly.validate(out))throw new Error('compiler emitted invalid wasm');
    const guest=new WebAssembly.Module(out);
    // atto.browser.guest.v1: reject forged function signatures, not only names.
    validateGuestImportABI(out, guest);
    let stdout='',instance;
    const state=createStateRuntime();
    const atto={
      print_i64:v=>{stdout+=String(v)},
      print_bool:v=>{stdout+=v?'true':'false'},
      print_str:(p,n)=>{stdout+=new TextDecoder('utf-8',{fatal:true}).decode(new Uint8Array(instance.exports.memory.buffer,p,n))},
      print_newline:()=>{stdout+='\n'},
      print_f64:v=>{stdout+=javaDouble(v)},
      ...state.imports,
    };
    instance=new WebAssembly.Instance(guest,{atto});
    let value;
    try{value=instance.exports.main()}
    catch(e){
      // The synchronous WebAssembly call has physically unwound. Release
      // abandoned state leases only here, never while guest code still runs.
      state.abort();
      const code=instance.exports.panic_code.value;
      const kind=code===1?'integer overflow':code===2?'division by zero':code===3?'heap exhausted':code===6?'stack overflow (logical call-depth quota)':'trap: '+e.message;
      self.postMessage({ok:false,error:stdout+'E_PANIC: '+kind});return;
    }
    state.complete(); // rejects any scope that failed to release its lease
    self.postMessage({ok:true,value:String(value),stdout});
  }catch(e){self.postMessage({ok:false,error:'E_HOST: '+e.message})}
};

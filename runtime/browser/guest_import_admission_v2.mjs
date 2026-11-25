// Shared, fail-closed binary Wasm import validation for audited browser/JSPI hosts.
// This module does NOT grant imports: host must provide matching implementations.
// Copied from the hardened browser Worker in PR #241 and extended for negotiated JSPI.

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
  sleep_ms: [[0x7e], [0x7e]],
  barrier: [[0x7e], [0x7e]],
  task_start: [[0x7f], [0x7f]],
  task_await: [[0x7f], [0x7e]],
  cooperate: [[], []],
  channel_int_nb_read_v1: [[0x7f, 0x7f], [0x7f]],
  channel_int_await_read_v1: [[0x7f, 0x7f], [0x7f]],
});
function abiReject(reason) { throw new Error('E_ABI: ' + reason); }
const abiDecoder = new TextDecoder('utf-8', {fatal: true});

const BASE = Object.freeze(['print_i64','print_bool','print_str','print_newline',
  'print_f64','state_register','state_enter','state_leave']);
const ASYNC = Object.freeze(['sleep_ms','barrier','task_start','task_await','cooperate']);
const CHANNEL = Object.freeze(['channel_int_nb_read_v1','channel_int_await_read_v1']);
function marker(module, label, expected) {
  const entries = WebAssembly.Module.customSections(module, label);
  if (entries.length === 0) return false;
  if (entries.length !== 1 || !(
      new Uint8Array(entries[0]).length === 1 && new Uint8Array(entries[0])[0] === expected))
    abiReject('invalid or repeated version marker ' + label);
  return true;
}
export function validateGuestImportABI(bytes, module = null, policy = 'browser-basic-v1') {
  if (!['browser-basic-v1','browser-async-v2','channel-int-v1'].includes(policy))
    abiReject('unsupported host policy');
  const permitted = new Set(BASE);
  if (policy === 'browser-async-v2') ASYNC.forEach(x=>permitted.add(x));
  if (policy === 'channel-int-v1') ['cooperate',...CHANNEL].forEach(x=>permitted.add(x));
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
        if (mod !== 'atto' || !permitted.has(symbol) || !Object.hasOwn(ABI_SIGNATURES, symbol))
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
    const hasAsync=marker(module,'atto.async',2);
    const hasChannel=marker(module,'atto.channel_int',1);
    if (policy === 'browser-basic-v1' && (hasAsync || hasChannel))
      abiReject('v1 browser cannot grant asynchronous imports');
    if (policy === 'browser-async-v2' && hasChannel)
      abiReject('ordinary async Worker cannot grant channel handles');
    if (policy === 'browser-async-v2' && imported.some(x=>ASYNC.includes(x.name)) && !hasAsync)
      abiReject('missing async v2 marker');
    if (policy === 'channel-int-v1' && (!hasAsync || !hasChannel ||
      CHANNEL.some(x=>!imported.some(i=>i.name===x)) ||
      !imported.some(i=>i.name==='cooperate')))
      abiReject('missing mandatory channel JSPI signatures or version markers');
    const reflected = WebAssembly.Module.imports(module);
    if (imported.length !== reflected.length || imported.some((entry, i) =>
      entry.module !== reflected[i].module || entry.name !== reflected[i].name ||
      entry.kind !== reflected[i].kind)) abiReject('binary/reflection import mismatch');
  }
  if (!module && policy !== 'browser-basic-v1')
    abiReject('negotiated ABI requires validated WebAssembly.Module for custom marker inspection');
  return imported;
}


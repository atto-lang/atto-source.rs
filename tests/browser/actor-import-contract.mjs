// Browser guest Wasm import ABI (private source-owned compiler slice).
// Exact module/member/kind/uniqueness AND exact function parameter/result types.
// Validate against raw bytes, never an unbound Module+different-byte pairing.
export const ACTOR_GUEST_IMPORT_SIGNATURES = Object.freeze({
  print_i64:    Object.freeze({params:Object.freeze([0x7e]),       results:Object.freeze([])}),
  print_bool:   Object.freeze({params:Object.freeze([0x7f]),       results:Object.freeze([])}),
  print_str:    Object.freeze({params:Object.freeze([0x7f,0x7f]),  results:Object.freeze([])}),
  print_newline:Object.freeze({params:Object.freeze([]),           results:Object.freeze([])}),
  actor_spawn:  Object.freeze({params:Object.freeze([0x7f]),       results:Object.freeze([0x7f])}),
  actor_send:   Object.freeze({params:Object.freeze([0x7f,0x7e]),  results:Object.freeze([0x7f])}),
  actor_join:   Object.freeze({params:Object.freeze([0x7f]),       results:Object.freeze([])}),
});
export const ACTOR_GUEST_IMPORTS = Object.freeze(Object.keys(ACTOR_GUEST_IMPORT_SIGNATURES));
const allowed = new Set(ACTOR_GUEST_IMPORTS);
const validValueTypes = new Set([0x7f,0x7e]); // i32, i64 only
const utf8 = new TextDecoder('utf-8',{fatal:true});

function abi_error(code) { return new Error(code); }

function inspect_function_import_signatures(bytes) {
  const data = bytes instanceof Uint8Array ? bytes :
    bytes instanceof ArrayBuffer ? new Uint8Array(bytes) : null;
  if (!data) throw abi_error('E_ACTOR_IMPORT_BYTES');
  if (data.length < 8 ||
      data[0]!==0 || data[1]!==97 || data[2]!==115 || data[3]!==109 ||
      data[4]!==1 || data[5]!==0 || data[6]!==0 || data[7]!==0) {
    throw abi_error('E_ACTOR_IMPORT_BINARY');
  }
  let at = 8, types = null, imports = [];
  const readByte = end => {
    if (at >= end) throw abi_error('E_ACTOR_IMPORT_BINARY');
    return data[at++];
  };
  const u32 = end => {
    let v = 0;
    for (let i = 0; i < 5; i++) {
      const b = readByte(end);
      if (i===4 && (b & 0xf0)) throw abi_error('E_ACTOR_IMPORT_BINARY');
      v += (b & 0x7f) * 2**(7*i);
      if (!(b & 0x80)) {
        if (i && b===0) throw abi_error('E_ACTOR_IMPORT_BINARY');
        return v;
      }
    }
    throw abi_error('E_ACTOR_IMPORT_BINARY');
  };
  const name = end => {
    const size = u32(end);
    if (size > 256 || at+size > end) throw abi_error('E_ACTOR_IMPORT_BINARY');
    const segment = data.subarray(at,at+size);
    at += size;
    try { return utf8.decode(segment); }
    catch { throw abi_error('E_ACTOR_IMPORT_BINARY'); }
  };
  const values = end => {
    const count = u32(end);
    if (count > 32) throw abi_error('E_ACTOR_IMPORT_SIGNATURE');
    const out = [];
    for (let i=0; i<count; i++) {
      const value = readByte(end);
      if (!validValueTypes.has(value)) throw abi_error('E_ACTOR_IMPORT_SIGNATURE');
      out.push(value);
    }
    return out;
  };
  while (at < data.length) {
    const id = readByte(data.length);
    const size = u32(data.length), end = at + size;
    if (end > data.length) throw abi_error('E_ACTOR_IMPORT_BINARY');
    if (id===1) {
      if (types) throw abi_error('E_ACTOR_IMPORT_BINARY');
      const count = u32(end);
      if (count > 4096) throw abi_error('E_ACTOR_IMPORT_SIGNATURE');
      types = [];
      for (let i=0;i<count;i++) {
        if (readByte(end)!==0x60) throw abi_error('E_ACTOR_IMPORT_SIGNATURE');
        types.push({params:values(end),results:values(end)});
      }
    } else if (id===2) {
      if (imports.length) throw abi_error('E_ACTOR_IMPORT_BINARY');
      const count = u32(end);
      if (count > ACTOR_GUEST_IMPORTS.length) throw abi_error('E_ACTOR_IMPORT_COUNT');
      imports = [];
      for (let i=0;i<count;i++) {
        const module = name(end), field = name(end);
        const kind = readByte(end);
        if (kind!==0) throw abi_error('E_ACTOR_IMPORT_KIND');
        imports.push({module,field,typeIndex:u32(end)});
      }
    }
    if (id===1 || id===2) {
      if (at!==end) throw abi_error('E_ACTOR_IMPORT_BINARY');
    } else {
      at=end;
    }
  }
  if (imports.length && !types) throw abi_error('E_ACTOR_IMPORT_BINARY');
  const declared = [];
  for (const imp of imports) {
    if (imp.module!=='atto' || !allowed.has(imp.field)) {
      throw abi_error('E_ACTOR_IMPORT_NAME');
    }
    const actual = types[imp.typeIndex], expected = ACTOR_GUEST_IMPORT_SIGNATURES[imp.field];
    if (!actual ||
        actual.params.length!==expected.params.length ||
        actual.results.length!==expected.results.length ||
        actual.params.some((v,i)=>v!==expected.params[i]) ||
        actual.results.some((v,i)=>v!==expected.results[i])) {
      throw abi_error('E_ACTOR_IMPORT_SIGNATURE');
    }
    declared.push({module:imp.module,name:imp.field,kind:'function'});
  }
  return declared;
}

/**
 * Validates exact imports for the supplied bytes, not caller-supplied metadata.
 * Only use on bytes that are verified by WebAssembly.Module; this function
 * additionally compiles them in the browser engine before accepting their ABI.
 */
export function validate_actor_guest_imports(wasmBytes) {
  if (!(wasmBytes instanceof Uint8Array || wasmBytes instanceof ArrayBuffer)) {
    throw abi_error('E_ACTOR_IMPORT_BYTES');
  }
  // Browser engine validates the entire module, including unknown sections;
  // this bounded parser reads only type/import declarations.
  const module = new WebAssembly.Module(wasmBytes);
  const imports = WebAssembly.Module.imports(module);
  if (imports.length > ACTOR_GUEST_IMPORTS.length) {
    throw abi_error('E_ACTOR_IMPORT_COUNT');
  }
  const seen = new Set();
  for (const imp of imports) {
    if (imp.module!=='atto' || !allowed.has(imp.name)) {
      throw abi_error('E_ACTOR_IMPORT_NAME');
    }
    if (imp.kind!=='function') throw abi_error('E_ACTOR_IMPORT_KIND');
    if (seen.has(imp.name)) throw abi_error('E_ACTOR_IMPORT_DUPLICATE');
    seen.add(imp.name);
  }
  const declared = inspect_function_import_signatures(wasmBytes);
  if (declared.length!==imports.length ||
      declared.some((imp,i)=>imp.module!==imports[i].module ||
        imp.name!==imports[i].name || imp.kind!==imports[i].kind)) {
    throw abi_error('E_ACTOR_IMPORT_BINARY');
  }
  return imports;
}

// Dedicated browser worker compiling real Atto actor source to real guest Wasm.
// This is an executable scalar receive() prototype, not the full actor runtime.
let actor = null;
let initializing = false;
let terminal = false;
let nextMessageId = 1;
function error(reason) {
  if (terminal) return;
  terminal = true;
  self.postMessage({ kind: 'fault', message: String(reason).slice(0, 512) });
  self.close();
}
self.onmessage = async ({ data }) => {
  if (terminal) return;
  if (!actor) {
    // A second init or a message arriving while fetch/compile is pending
    // must never start another compiler instance or overwrite this actor.
    if (initializing || data?.kind !== 'init' || typeof data.source !== 'string') {
      return error('E_ACTOR_PROTOCOL: expected exactly one init');
    }
    initializing = true;
    try {
      const response = await fetch('/atto_compiler.wasm');
      if (!response.ok) throw new Error('compiler asset unavailable');
      const bytes = await response.arrayBuffer();
      if (!WebAssembly.validate(bytes)) throw new Error('invalid compiler Wasm');
      const compiler = new WebAssembly.Instance(new WebAssembly.Module(bytes), {}).exports;
      if (typeof compiler.compile_actor !== 'function') throw new Error('missing actor compiler ABI');
      const source = new TextEncoder().encode(data.source);
      if (source.byteLength > compiler.input_capacity()) throw new Error('actor source too large');
      new Uint8Array(compiler.memory.buffer, compiler.input_ptr(), source.length).set(source);
      if (compiler.compile_actor(source.length) !== 0) {
        if (compiler.result_length() !== 0) throw new Error('actor rejected but produced an artifact');
        const memory = new Uint8Array(compiler.memory.buffer);
        let end = compiler.error_ptr();
        while (end < memory.length && memory[end] && end - compiler.error_ptr() < 512) end++;
        throw new Error(new TextDecoder().decode(memory.subarray(compiler.error_ptr(), end)));
      }
      const guestBytes = new Uint8Array(compiler.memory.buffer, compiler.result_ptr(), compiler.result_length()).slice();
      if (guestBytes.byteLength > (1 << 20) || !WebAssembly.validate(guestBytes)) throw new Error('invalid actor guest');
      const module = new WebAssembly.Module(guestBytes);
      if (WebAssembly.Module.imports(module).length) throw new Error('actor guest has imports');
      const exports = WebAssembly.Module.exports(module);
      if (exports.length !== 1 || exports[0].name !== 'receive' || exports[0].kind !== 'function') {
        throw new Error('invalid actor export surface');
      }
      actor = new WebAssembly.Instance(module, {});
      self.postMessage({ kind: 'ready' });
    } catch (e) { error('E_ACTOR_INIT: ' + e.message); }
    return;
  }
  if (data?.kind !== 'message' || typeof data.value !== 'bigint' ||
      !Number.isSafeInteger(data.id) || data.id !== nextMessageId) {
    return error('E_ACTOR_PROTOCOL: skipped/replayed actor sequence');
  }
  // Validate the sequence *before* touching the guest instance.
  nextMessageId++;
  try {
    const value = actor.exports.receive(data.value);
    if (typeof value !== 'bigint') throw new Error('incorrect receive result');
    self.postMessage({ kind: 'result', id: data.id, value });
  } catch (e) { error('E_ACTOR_PANIC: ' + e.message); }
};

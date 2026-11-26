// Trusted, synchronous host for the optional atto.browser.guest.v2 span imports.
// Create one recorder per guest instance and bind its exported memory before invoking it.
const maxDepth = 64;
const maxReceipts = 1024;
const maxToken = (1n << 63n) - 1n;

export function createScopedTelemetry() {
  const decoder = new TextDecoder("utf-8", { fatal: true, ignoreBOM: true });
  const active = [];
  let completed = [];
  let memory;
  let enabled = false;
  let nextToken = 1n;
  let dropped = 0;

  function drop() {
    if (dropped < Number.MAX_SAFE_INTEGER) dropped++;
    return 0n;
  }

  function trace_span_begin(offset, length) {
    if (!enabled) return 0n;
    if (!memory || active.length === maxDepth || nextToken > maxToken ||
        offset < 0 || length < 1 || length > 256 || offset + length > memory.buffer.byteLength) {
      return drop();
    }

    let operation;
    try {
      operation = decoder.decode(new Uint8Array(memory.buffer, offset, length));
    } catch {
      return drop();
    }
    if (/[\u0000-\u001f\u007f-\u009f]/u.test(operation)) return drop();
    const span = { id: nextToken++, parentId: active.at(-1)?.id ?? 0n, site: offset, operation };
    active.push(span);
    return span.id;
  }

  function finish(disposition) {
    const span = active.pop();
    if (completed.length === maxReceipts) {
      drop();
    } else {
      completed.push({ ...span, disposition });
    }
  }

  function trace_span_end(token) {
    if (token === 0n) return;
    if (active.at(-1)?.id === token) finish("closed");
    else drop();
  }

  return {
    imports: { trace_span_begin, trace_span_end },
    bindMemory(guestMemory) {
      if (memory || !(guestMemory instanceof WebAssembly.Memory)) {
        throw new TypeError("bind the recorder once to its guest memory");
      }
      memory = guestMemory;
    },
    setEnabled(recording) {
      enabled = recording === true;
    },
    invoke(entry, ...args) {
      const depth = active.length;
      try {
        return entry(...args);
      } finally {
        // A fatal engine trap bypasses Wasm finally. Retire the host frames
        // from this invocation and preserve the original guest outcome.
        while (active.length > depth) finish("abandoned");
      }
    },
    drain() {
      const result = { completed, active: active.length, dropped };
      completed = [];
      dropped = 0;
      return result;
    },
  };
}

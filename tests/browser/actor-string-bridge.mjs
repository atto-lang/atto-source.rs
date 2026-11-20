// Bounded worker-memory DATA string bridge for the v1 actor Wasm ABI.
// Host runtime primitive ONLY. Does not enable ActorMail<string> in the compiler.
// The compiler/host must attest the receiver's reserved nonoverlapping memory.
const MAX_STRING_BYTES = 65536;
const HEADER_BYTES = 10;
const MAGIC = [0x41, 0x54, 0x41, 0x4d]; // ATAM
const utf8 = new TextDecoder('utf-8', { fatal: true, ignoreBOM: true });

export class ActorStringBridgeError extends Error {
  constructor(code) { super(code); this.name = 'ActorStringBridgeError'; this.code = code; }
}
const deny = code => { throw new ActorStringBridgeError(code); };
const uint32 = n => Number.isSafeInteger(n) && n >= 0 && n <= 0xffffffff;
function memoryView(memory) {
  if (!(memory instanceof WebAssembly.Memory)) deny('E_ACTOR_MEMORY');
  return new Uint8Array(memory.buffer);
}
function range(view, start, size) {
  if (!uint32(start) || !uint32(size) || start > view.byteLength || size > view.byteLength - start) {
    deny('E_ACTOR_BOUNDS');
  }
}

// Guest strings are the existing i64 (unsigned offset:u32 << 32 | len:u32).
export function unpackGuestString(value) {
  if (typeof value !== 'bigint') deny('E_ACTOR_STRING_LAYOUT');
  const bits = BigInt.asUintN(64, value);
  return { offset: Number(bits >> 32n), length: Number(bits & 0xffffffffn) };
}
function packGuestString(offset, length) {
  if (!uint32(offset) || !uint32(length)) deny('E_ACTOR_STRING_LAYOUT');
  return BigInt.asIntN(64, (BigInt(offset) << 32n) | BigInt(length));
}
function encodeData(payload) {
  if (!(payload instanceof Uint8Array)) deny('E_ACTOR_PAYLOAD');
  if (payload.byteLength > MAX_STRING_BYTES) deny('E_ACTOR_QUOTA');
  // No normalization, replacement, partial UTF-8, or implicit CONTROL routing.
  try { utf8.decode(payload); } catch { deny('E_ACTOR_UTF8'); }
  const frame = new Uint8Array(HEADER_BYTES + payload.length);
  frame.set(MAGIC);
  frame[4] = 1;
  frame[5] = 1; // DATA / string. CONTROL has no representation here.
  new DataView(frame.buffer).setUint32(6, payload.length, true);
  frame.set(payload, HEADER_BYTES);
  return frame;
}
export function decodeStringDataFrame(raw) {
  if (!(raw instanceof Uint8Array)) deny('E_ACTOR_FRAME');
  // Take ownership before parsing an attacker-controlled buffer; the caller
  // cannot race modifications into the payload after successful validation.
  if (raw.length < HEADER_BYTES) deny('E_ACTOR_TRUNCATED');
  if (raw.length > HEADER_BYTES + MAX_STRING_BYTES) deny('E_ACTOR_QUOTA');
  const bytes = Uint8Array.from(raw);
  if (!MAGIC.every((value, i) => bytes[i] === value) || bytes[4] !== 1 || bytes[5] !== 1) {
    deny('E_ACTOR_FRAME');
  }
  const len = new DataView(bytes.buffer).getUint32(6, true);
  if (len > MAX_STRING_BYTES) deny('E_ACTOR_QUOTA');
  if (len > bytes.length - HEADER_BYTES) deny('E_ACTOR_TRUNCATED');
  if (len < bytes.length - HEADER_BYTES) deny('E_ACTOR_TRAILING');
  try { utf8.decode(bytes.subarray(HEADER_BYTES)); } catch { deny('E_ACTOR_UTF8'); }
  return bytes;
}

// All handles are runtime-minted object capabilities held in a private WeakMap.
// No actor code can synthesize a sender/receiver by providing its ID or name.
export class ActorStringMailBridge {
  #actors = new WeakMap();
  #registered = new Map();
  #memoryOwners = new WeakMap();
  #maxMessages;
  #maxQueuedBytes;

  constructor({ maxMessages = 32, maxQueuedBytes = 131072 } = {}) {
    if (!Number.isSafeInteger(maxMessages) || maxMessages < 1 || maxMessages > 1024 ||
        !Number.isSafeInteger(maxQueuedBytes) || maxQueuedBytes < HEADER_BYTES ||
        maxQueuedBytes > 1024 * 1024) deny('E_ACTOR_QUOTA');
    this.#maxMessages = maxMessages;
    this.#maxQueuedBytes = maxQueuedBytes;
  }

  register({ tenant, actorId, generation, memory, receives, sends = [],
             reservation = null, guestDataEnd = null, actorType = null } = {}) {
    if (typeof tenant !== 'string' || !tenant ||
        !(typeof actorId === 'string' || uint32(actorId)) ||
        !Number.isSafeInteger(generation) || generation < 1 ||
        !Array.isArray(sends) || sends.some(t => t !== 'string' && t !== 'int' && t !== 'bool') ||
        !['string', 'int', 'bool', null].includes(receives)) deny('E_ACTOR_METADATA');
    const view = memoryView(memory);
    // Per-instance Wasm memory is never reusable by another live or retired actor.
    if (this.#memoryOwners.has(memory)) deny('E_ACTOR_MEMORY_ALIAS');
    const id = tenant.length + ':' + tenant + ':' + String(actorId).length + ':' + actorId;
    if (this.#registered.has(id)) deny('E_ACTOR_DUPLICATE');
    if (receives === 'string') {
      // A receiver reservation must be supplied by a trusted compiler manifest,
      // not inferred from a guest-controlled exported pointer.
      if (!reservation || !Number.isSafeInteger(guestDataEnd) || guestDataEnd < 0 ||
          !uint32(reservation.offset) || !uint32(reservation.capacity) ||
          reservation.capacity < 1 || reservation.capacity > MAX_STRING_BYTES ||
          reservation.offset < guestDataEnd) deny('E_ACTOR_RESERVATION');
      range(view, reservation.offset, reservation.capacity);
    } else if (reservation !== null) deny('E_ACTOR_RESERVATION');
    const handle = Object.freeze(Object.create(null));
    this.#actors.set(handle, {
      id, tenant, actorId, generation, memory, receives, sends: new Set(sends),
      reservation: reservation === null ? null : {offset: reservation.offset, capacity: reservation.capacity},
      actorType, closed: false, inDelivery: false, queue: [], queuedBytes: 0, nextSequence: 0,
    });
    this.#registered.set(id, handle);
    this.#memoryOwners.set(memory, handle);
    return handle;
  }

  // Called synchronously from the trusted actor_send import. Admission is
  // distinct from completion; data is copied before the sender can mutate it.
  sendPacked(sender, receiver, packed, {kind = 'DATA'} = {}) {
    const src = this.#actors.get(sender);
    const dst = this.#actors.get(receiver);
    if (!src || !dst) deny('E_ACTOR_CAPABILITY');
    if (kind !== 'DATA') deny('E_ACTOR_CONTROL_FORGED');
    if (src.closed || dst.closed) deny('E_ACTOR_CLOSED');
    if (src.tenant !== dst.tenant) deny('E_ACTOR_TENANT');
    if (!src.sends.has('string') || dst.receives !== 'string') deny('E_ACTOR_TYPE');
    if (!dst.reservation) deny('E_ACTOR_RESERVATION');
    const {offset, length} = unpackGuestString(packed);
    if (length > MAX_STRING_BYTES || length > dst.reservation.capacity) deny('E_ACTOR_QUOTA');
    const bytes = memoryView(src.memory);
    range(bytes, offset, length);
    const frame = encodeData(bytes.slice(offset, offset + length));
    if (dst.queue.length >= this.#maxMessages ||
        frame.length > this.#maxQueuedBytes - dst.queuedBytes) {
      frame.fill(0);
      deny('E_ACTOR_FULL');
    }
    if (!Number.isSafeInteger(dst.nextSequence + 1)) {
      frame.fill(0);
      deny('E_ACTOR_SEQUENCE');
    }
    const sequence = dst.nextSequence++;
    dst.queue.push({ frame, sequence, generation: dst.generation });
    dst.queuedBytes += frame.length;
    return Object.freeze({ status: 'ADMITTED', sequence });
  }

  // Delivers one frame into the receiver's *own* reserved Wasm memory and
  // invokes actor_receive(type, packed_ptr_len, seq). Any trap is terminal;
  // the host must additionally report a runtime-minted supervisor CONTROL.
  receiveNext(receiver, actor_receive) {
    const dst = this.#actors.get(receiver);
    if (!dst) deny('E_ACTOR_CAPABILITY');
    if (dst.closed) deny('E_ACTOR_CLOSED');
    if (dst.inDelivery) deny('E_ACTOR_BUSY');
    if (typeof actor_receive !== 'function') deny('E_ACTOR_HANDLER');
    const entry = dst.queue.shift();
    if (!entry) return null;
    dst.queuedBytes -= entry.frame.length;
    const {frame} = entry;
    dst.inDelivery = true;
    let copied = 0;
    try {
      if (entry.generation !== dst.generation) deny('E_ACTOR_GENERATION');
      const parsed = decodeStringDataFrame(frame);
      const payload = parsed.subarray(HEADER_BYTES);
      const view = memoryView(dst.memory);
      range(view, dst.reservation.offset, dst.reservation.capacity);
      if (payload.length > dst.reservation.capacity) deny('E_ACTOR_QUOTA');
      copied = payload.length;
      view.set(payload, dst.reservation.offset);
      const packed = packGuestString(dst.reservation.offset, payload.length);
      actor_receive(dst.actorType, packed, BigInt(entry.sequence));
      if (dst.closed || entry.generation !== dst.generation) deny('E_ACTOR_CLOSED');
      return Object.freeze({ status: 'HANDLER_RETURNED', sequence: entry.sequence });
    } catch (error) {
      // Invalidation is terminal; no retry with stale memory or generation.
      dst.closed = true;
      this.#registered.delete(dst.id);
      for (const queued of dst.queue) queued.frame.fill(0);
      dst.queue.length = 0;
      dst.queuedBytes = 0;
      throw error;
    } finally {
      dst.inDelivery = false;
      if (copied > 0) {
        const view = memoryView(dst.memory);
        if (dst.reservation.offset <= view.length &&
            copied <= view.length - dst.reservation.offset) {
          view.fill(0, dst.reservation.offset, dst.reservation.offset + copied);
        }
      }
      frame.fill(0);
    }
  }

  close(handle) {
    const dst = this.#actors.get(handle);
    if (!dst) deny('E_ACTOR_CAPABILITY');
    if (dst.closed) return false;
    dst.closed = true;
    this.#registered.delete(dst.id);
    for (const queued of dst.queue) queued.frame.fill(0);
    dst.queue.length = 0;
    dst.queuedBytes = 0;
    return true;
  }
}

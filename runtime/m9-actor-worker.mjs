// M9 dedicated-worker bridge for the restricted, compiled .atto M8 guest ABI.
// Runs identically in a browser Worker or a Node worker_threads Worker.
// This is NOT the normal atto.browser.guest.v2 actor runtime.
const isBrowser = typeof self !== 'undefined' && typeof self.postMessage === 'function';
const port = isBrowser ? self : (await import('node:worker_threads')).parentPort;
let actor_id = null, generation = null, module = null, guest = null, active = null;
function respond(message) { port.postMessage({actor_id, generation, ...message}); }
function receive(cb) {
  if (isBrowser) port.addEventListener('message', e => cb(e.data));
  else port.on('message', cb);
}
function fail_if(condition, reason) { if (condition) throw new Error(reason); }
const expected = ['m8_new','m8_resume','m8_result','m8_pc','m8_close','m8_index','m8_fault'];
receive(message => {
  try {
    if (message?.type === 'INIT') {
      fail_if(guest !== null, 'double initialization');
      fail_if(!Number.isSafeInteger(message.actor_id) || message.actor_id < 0, 'invalid actor id');
      fail_if(!Number.isSafeInteger(message.generation) || message.generation < 1, 'invalid generation');
      const bytes = message.bytes;
      fail_if(!(bytes instanceof Uint8Array) && !(bytes instanceof ArrayBuffer), 'invalid guest bytes');
      fail_if(!WebAssembly.validate(bytes), 'invalid compiled guest');
      module = new WebAssembly.Module(bytes);
      fail_if(WebAssembly.Module.imports(module).length !== 0, 'compiled guest has forbidden imports');
      const exports = WebAssembly.Module.exports(module).filter(x => x.kind === 'function').map(x => x.name);
      fail_if(expected.some(x => !exports.includes(x)), 'missing compiled M8 ABI');
      fail_if(WebAssembly.Module.customSections(module,'atto.m8.cooperate').length !== 1,
        'guest is not the M8 cooperative profile');
      actor_id = message.actor_id; generation = message.generation;
      guest = new WebAssembly.Instance(module, {}).exports;
      respond({type:'READY'});
      return;
    }
    fail_if(guest === null, 'guest not initialized');
    fail_if(message.actor_id !== actor_id || message.generation !== generation, 'wrong worker generation');
    if (message.type === 'BEGIN') {
      fail_if(active !== null, 'actor receive overlap');
      fail_if(!Number.isSafeInteger(message.message_id) || message.message_id < 1, 'invalid message id');
      const amount = BigInt(message.amount);
      const handle = guest.m8_new(amount);
      active = {message_id:message.message_id, handle};
      respond({type:'BEGUN', message_id:active.message_id});
      return;
    }
    if (message.type === 'SLICE') {
      fail_if(!active || message.message_id !== active.message_id, 'stale continuation');
      const id = active.message_id;
      let fault = 0;
      try {
        const outcome = guest.m8_resume(active.handle, message.budget);
        if (outcome === 1) {
          respond({type:'YIELD', message_id:id, pc:guest.m8_pc(active.handle)});
        } else if (outcome === 0) {
          const result = guest.m8_result(active.handle);
          guest.m8_close(active.handle);
          active = null;
          respond({type:'DONE',message_id:id,result:String(result)});
        } else {
          throw new Error('forged continuation status');
        }
      } catch (error) {
        try { fault = guest.m8_fault(active.handle); } catch {}
        // Failed frame is quiescent after an exported Wasm call traps.
        // Close before publishing failure, so a subsequent actor generation
        // cannot regain old-frame authority.
        try { guest.m8_close(active.handle); } catch {}
        active = null;
        respond({type:'FAILED',message_id:id,fault_code:fault || 101,reason:fault?
          'checked guest arithmetic' : 'guest protocol violation'});
      }
      return;
    }
    if (message.type === 'CANCEL') {
      if (active !== null) {
        try { guest.m8_close(active.handle); } catch {}
        active = null;
      }
      respond({type:'CLOSED',message_id:message.message_id ?? 0});
      return;
    }
    throw new Error('unrecognized worker command');
  } catch (e) {
    respond({type:'PROTOCOL_ERROR',message_id:message?.message_id ?? 0,reason:String(e.message || e)});
  }
});

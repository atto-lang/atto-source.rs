// Parent-side execution watchdog. This module runs outside both the compiler
// worker and any actor workers. A synchronous runaway Wasm loop cannot block
// this timer because it executes on the page's separate event loop.
export const RUN_DEADLINE_MS = 5000;
export const MAX_RUN_OUTPUT_CHARS = 16384;
export const MAX_RUN_ERROR_CHARS = 256;

export class BrowserRunSession {
  constructor(worker, request, on_settle, { deadline_ms = RUN_DEADLINE_MS } = {}) {
    if (!worker || typeof worker.postMessage !== 'function' ||
        typeof worker.terminate !== 'function' ||
        typeof on_settle !== 'function' ||
        !Number.isSafeInteger(deadline_ms) || deadline_ms < 1 ||
        deadline_ms > RUN_DEADLINE_MS) {
      throw new Error('E_RUN_SESSION_ARGS');
    }
    this.worker = worker;
    this.state = 'running';
    this.on_settle = on_settle;
    this.timer = setTimeout(() => this.finish({ok:false,error:'E_TIMEOUT'}),deadline_ms);
    worker.onmessage = event => this.finish(event?.data);
    worker.onerror = () => this.finish({ok:false,error:'E_WORKER'});
    worker.onmessageerror = () => this.finish({ok:false,error:'E_WORKER_MESSAGE'});
    try { worker.postMessage(request); }
    catch { this.finish({ok:false,error:'E_WORKER_CLONE'}); }
  }

  finish(response) {
    if (this.state !== 'running') return false;
    this.state = 'completed';
    clearTimeout(this.timer);
    // Tear down before reporting the result: stale events cannot overwrite a
    // timeout or a completed run, even when they were queued before terminate.
    this.worker.onmessage = null;
    this.worker.onerror = null;
    this.worker.onmessageerror = null;
    try { this.worker.terminate(); } catch { /* already exiting */ }
    let outcome={ok:false,error:'E_WORKER_PROTOCOL'};
    if (response && typeof response === 'object') {
      if (response.ok === true &&
          typeof response.value === 'string' &&
          response.value.length <= 40 &&
          typeof response.stdout === 'string' &&
          response.stdout.length <= MAX_RUN_OUTPUT_CHARS) {
        outcome={ok:true,value:response.value,stdout:response.stdout};
      } else if (response.ok === false &&
                 typeof response.error === 'string' &&
                 response.error.length <= MAX_RUN_ERROR_CHARS) {
        outcome={ok:false,error:response.error};
      }
    }
    this.outcome=outcome;
    this.on_settle(outcome);
    return true;
  }

  cancel() {
    return this.finish({ok:false,error:'E_CANCELLED'});
  }
}

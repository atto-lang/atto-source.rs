// Versioned host-side JSPI wake bridge for the atto.async/2 cooperative import.
// This does NOT provide portable task_resume frames, or resume Wasm within a
// synchronous scheduler quantum: JSPI resumes it via a later microtask.
export const JSPI_CONTROL_BRIDGE_V1 = 'atto.browser.jspi.control.v1';
const positiveInt = n => Number.isSafeInteger(n) && n >= 1 && n <= 65536;
function verifyCooperativeGuest(module) {
  if (!(module instanceof WebAssembly.Module)) throw new TypeError('E_ABI_MODULE');
  const markers=WebAssembly.Module.customSections(module,'atto.async');
  if (markers.length!==1 || new Uint8Array(markers[0]).length!==1 ||
      new Uint8Array(markers[0])[0]!==2) throw new TypeError('E_ABI_ASYNC_VERSION');
  const imports=WebAssembly.Module.imports(module).filter(x=>
    x.module==='atto' && x.name==='cooperate' && x.kind==='function');
  if (imports.length!==1) throw new TypeError('E_ABI_COOPERATE_IMPORT');
}
export function createJspiControlBridgeV1({
  loop,owner,epoch,maxPending=256,
  scheduleWake=callback=>setTimeout(callback,0),
  cancelWake=timer=>clearTimeout(timer)
}={}) {
  if (!loop || typeof loop.attachOwner!=='function' ||
      typeof loop.beginSuspension!=='function' || typeof loop.complete!=='function' ||
      typeof loop.cancel!=='function' || typeof loop.retireOwner!=='function' ||
      !positiveInt(maxPending) || typeof scheduleWake!=='function' ||
      typeof cancelWake!=='function') throw new TypeError('E_BRIDGE_CONFIG');
  if (!loop.attachOwner(owner,epoch)) throw new Error('E_OWNER_ADMISSION');
  let closed=false,resolved=0,cancelled=0,rejected=0;
  const pending=new Set(),shutdownError=()=>new Error('E_OWNER_RETIRED');
  function stopEntry(entry,error) {
    if (!entry.live) return false;
    entry.live=false;
    pending.delete(entry);
    // Completion stays revocable until the scheduler dispatches its turn.
    if(entry.timer!==null) {
      try {cancelWake(entry.timer)} catch {/* revocation still wins */}
      entry.timer=null;
    }
    loop.cancel(entry.ticket);
    cancelled++;
    entry.reject(error);
    return true;
  }
  function cooperate() {
    if(closed) return Promise.reject(shutdownError());
    if(pending.size>=maxPending){
      rejected++;
      return Promise.reject(new Error('E_BRIDGE_QUOTA'));
    }
    return new Promise((resolve,reject)=>{
      const entry={ticket:null,timer:null,live:true,resolve,reject};
      const ticket=loop.beginSuspension(owner,epoch,()=>{
        if(!entry.live)return;
        entry.live=false;
        entry.timer=null;
        pending.delete(entry);
        resolved++;
        // The scheduler only resolves the Promise; JSPI executes guest later.
        resolve(undefined);
      });
      if(!ticket){rejected++;reject(new Error('E_BRIDGE_ADMISSION'));return;}
      entry.ticket=ticket;
      pending.add(entry);
      try{
        let synchronous=true,fired=false;
        const timer=scheduleWake(()=>{
          if(!entry.live||fired)return;
          fired=true;
          if(synchronous){stopEntry(entry,new Error('E_WAKE_NOT_ASYNC'));return;}
          entry.timer=null;
          try {
            if(!loop.complete(ticket,undefined))stopEntry(entry,new Error('E_STALE_WAKE'));
          } catch {stopEntry(entry,new Error('E_WAKE_FAILED'));}
        });
        synchronous=false;
        if(entry.live)entry.timer=timer;
      }catch{stopEntry(entry,new Error('E_WAKE_SCHEDULE'));}
    });
  }
  return Object.freeze({
    version:JSPI_CONTROL_BRIDGE_V1,owner,epoch,cooperate,
    makeCooperateImport(module){
      if(closed)throw shutdownError();
      verifyCooperativeGuest(module);
      if(typeof WebAssembly.Suspending!=='function')throw new Error('E_JSPI_UNAVAILABLE');
      return new WebAssembly.Suspending(()=>cooperate());
    },
    retire(){
      if(closed)return false;
      closed=true;
      for(const entry of [...pending])stopEntry(entry,shutdownError());
      loop.retireOwner(owner,epoch);
      return true;
    },
    get stats(){return Object.freeze({closed,pending:pending.size,resolved,cancelled,rejected});}
  });
}

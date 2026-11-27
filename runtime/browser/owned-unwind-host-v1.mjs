// Trusted adapter for Rust's failure-aware owned-unwind v1 guest ABI.
// Inject the same JS backend claimFutures and Worker control loop used by
// compiler-emitted JS. Not a guest capability or hostile artifact validator.
const usedInstances=new WeakSet();
const arities=new WeakMap();
const MARKER='atto.browser.owned_unwind.v1';
const NAMES=['start','pending','complete','resume','outcome','result','error','suppressed_count','suppressed_error','suppressed_rank','retire'];
function quantum(fn){
  if(typeof fn!=='function'||Object.prototype.toString.call(fn)!=='[object Function]')throw new TypeError('E_SYNC_QUANTUM_REQUIRED');
}
function ensure(ok,msg){if(!ok)throw Error(msg);}
export function instantiateOwnedUnwind(module){
  if(!(module instanceof WebAssembly.Module))throw TypeError('E_OWNED_UNWIND_MODULE');
  const markers=WebAssembly.Module.customSections(module,MARKER);
  const marker=markers.length===1?new Uint8Array(markers[0]):null;
  ensure(marker&&marker.length===2&&marker[0]===1&&marker[1]>=1&&marker[1]<=8,'E_OWNED_UNWIND_MARKER');
  ensure(WebAssembly.Module.imports(module).length===0,'E_OWNED_UNWIND_IMPORTS');
  const names=WebAssembly.Module.exports(module);
  ensure(names.length===NAMES.length&&names.every(n=>n.kind==='function'&&NAMES.some(k=>n.name==='unwind_'+k+'_v1')),'E_OWNED_UNWIND_EXPORTS');
  // ABI markers do not authenticate arbitrary hostile Wasm. Only trusted
  // compiler output may be instantiated by the embedding application.
  const instance=new WebAssembly.Instance(module);
  arities.set(instance,marker[1]);
  return instance;
}
export function startOwnedUnwind({instance,scheduler,runtime,ownerKey,owner,epoch,futures,onDone}){
  quantum(onDone);
  ensure(instance instanceof WebAssembly.Instance&&arities.has(instance)&&!usedInstances.has(instance),'E_OWNED_UNWIND_INSTANCE_USED');
  ensure(Number.isInteger(owner)&&owner>0&&owner<=0x7fffffff&&Number.isInteger(epoch)&&epoch>0&&epoch<=0x7fffffff,'E_OWNED_UNWIND_OWNER');
  ensure(Array.isArray(futures)&&futures.length===arities.get(instance),'E_OWNED_UNWIND_ARITY');
  const e=instance.exports;
  const call=(n,...args)=>e['unwind_'+n+'_v1'](owner,epoch,...args);
  // Arity is compiler-pinned and verified before any claim or admission.
  // The pending formal slot is independently range checked below.
  const claim=runtime.claimFutures(futures,arities.get(instance));
  let lease,admitted=false;
  const errors=[];
  function receipt(error){
    ensure(error instanceof Error,'E_OWNED_UNWIND_ERROR');
    let i=errors.indexOf(error);
    if(i<0){ensure(errors.length<8,'E_OWNED_UNWIND_ERROR_QUOTA');i=errors.push(error)-1;}
    return BigInt(i+1);
  }
  function errorAt(id){ensure(id>0n&&id<=BigInt(errors.length),'E_OWNED_UNWIND_ERROR_ID');return errors[Number(id)-1];}
  function terminal(){
    const k=call('outcome'),id=call('error');
    let outcome;
    if(k===1)outcome=Object.freeze({kind:'returned',value:call('result')});
    else if(k===2)outcome=Object.freeze({kind:'recovered',value:call('result'),error:errorAt(id)});
    else{
      ensure(k>=3&&k<=5,'E_OWNED_UNWIND_OUTCOME');
      const n=call('suppressed_count'),suppressed=[],kinds=[];
      ensure(n>=0&&n<=7,'E_OWNED_UNWIND_SUPPRESSED');
      for(let i=0;i<n;i++){
        suppressed.push(errorAt(call('suppressed_error',i)));
        const rank=call('suppressed_rank',i);
        ensure(rank>=1&&rank<=3,'E_OWNED_UNWIND_RANK');kinds.push(['','raise','panic','cancel'][rank]);
      }
      outcome=Object.freeze({kind:'unhandled',failure_kind:['raise','panic','cancel'][k-3],error:errorAt(id),suppressed:Object.freeze(suppressed),suppressed_kinds:Object.freeze(kinds),suppressed_overflow:0});
    }
    ensure(call('retire')===0,'E_OWNED_UNWIND_RETIRE');
    // Publication is inside the physical lease, even if host callback fails.
    try{const r=onDone(outcome);ensure(!r||typeof r.then!=='function','E_SYNC_QUANTUM_REQUIRED');}
    finally{errors.length=0;lease.close();}
  }
  function park(){
    const slot=call('pending');
    ensure(slot>=1&&slot<=claim.futures.length,'E_OWNED_UNWIND_SLOT');
    const future=claim.futures[slot-1];
    const ticket=scheduler.beginSuspension(ownerKey,epoch,completion=>{
      // Only the scheduled turn may touch guest exports or error identities.
      const {kind,value}=completion;
      ensure(call('complete',slot,kind,kind===0?value:receipt(value))===0,'E_OWNED_UNWIND_COMPLETE');
      const status=call('resume');
      if(status===1)park();else if(status===2)terminal();else throw Error('E_OWNED_UNWIND_RESUME');
    });
    ensure(ticket,'E_OWNED_UNWIND_SUSPENSION_ADMISSION');
    try{
      const cell=future.createResultCell();
      future.subscribeReady(sub=>{
        try{
          ensure(future.tryCommit(sub,cell),'E_OWNED_UNWIND_FUTURE_COMMIT');
          const outcome=cell.status==='value'?{kind:0,value:cell.value}:cell.status==='failed'?{kind:1,value:cell.error}:cell.status==='cancelled'?{kind:2,value:new Error('E_FUTURE_CANCELLED')}:null;
          ensure(outcome,'E_OWNED_UNWIND_FUTURE_STATUS');
          ensure(scheduler.complete(ticket,Object.freeze(outcome)),'E_OWNED_UNWIND_WAKE');
        }catch(error){sub.close();throw error;}
      });
    }catch(error){scheduler.cancel(ticket);throw error;}
  }
  try{
    lease=scheduler.retainOwner(ownerKey,epoch);
    if(!lease)return false;
    usedInstances.add(instance);
    admitted=scheduler.admitData(ownerKey,epoch,()=>{
      ensure(call('start')===1,'E_OWNED_UNWIND_START');park();
    });
    if(admitted)claim.commit();
    return admitted;
  }finally{
    if(!admitted){usedInstances.delete(instance);lease?.close();}
    claim.abort();
  }
  // Fatal scheduling/host invariant violations fail-stop and retain the
  // lease rather than falsely proving quiescence. STOP deadlines remain open.
}

// Experimental closed-world AOT guest-v3 host adapter.
// Calls one compiler-generated Wasm lifecycle export per bounded scheduler turn.
// Never invokes legacy main or WebAssembly.Suspending. Single instance/task only.
// Owner/epoch identify trusted host authority; NOT an authenticated wire protocol.
import {createWorkerControlLoop} from './worker-control-loop.mjs';
export const PORTABLE_TASK_V3='atto.browser.guest.v3';
const positive=x=>Number.isSafeInteger(x)&&x>0&&x<=0x7fffffff;
const INVALID=-1,READY=0,PENDING=1,DONE=2,CANCELLED=4;
const EXPORTS=['task_start_v3','task_resume_v3','task_cancel_v3','task_drop_v3','task_result_v3'];

export function verifyPortableTaskGuestV3(module) {
  if(!(module instanceof WebAssembly.Module))throw new TypeError('E_V3_MODULE');
  const v=WebAssembly.Module.customSections(module,PORTABLE_TASK_V3);
  if(v.length!==1||new Uint8Array(v[0]).length!==1||new Uint8Array(v[0])[0]!==3)
    throw new TypeError('E_V3_MARKER');
  const compat=WebAssembly.Module.customSections(module,'atto.async');
  if(compat.length!==1||new Uint8Array(compat[0]).length!==1||
      new Uint8Array(compat[0])[0]!==2)throw new TypeError('E_V3_COMPAT_MARKER');
  const imports=WebAssembly.Module.imports(module);
  if(imports.length!==1||imports[0].module!=='atto'||
     imports[0].name!=='cooperate'||imports[0].kind!=='function')
    throw new TypeError('E_V3_IMPORTS');
  const exports=WebAssembly.Module.exports(module);
  for(const name of EXPORTS){
    if(exports.filter(x=>x.name===name&&x.kind==='function').length!==1)
      throw new TypeError('E_V3_EXPORTS');
  }
  // This profile is exactly one fixed-size, private-global frame.
  if(exports.some(x=>x.kind==='memory'||x.kind==='table'||x.name==='task_invoke'))
    throw new TypeError('E_V3_SHAPE');
  return true;
}

export function createPortableTaskHostV3({
  module,loop=createWorkerControlLoop(),owner='portable-task',
  ownerId=1,epoch=1
}={}) {
  verifyPortableTaskGuestV3(module);
  if(!positive(ownerId)||!positive(epoch)||typeof owner!=='string'||
      !/^[A-Za-z0-9_-]{1,128}$/.test(owner))
    throw new TypeError('E_V3_IDENTITY');
  if(typeof loop?.attachOwner!=='function'||typeof loop?.admitData!=='function'||
     typeof loop?.admitControl!=='function'||typeof loop?.retireOwner!=='function')
    throw new TypeError('E_V3_LOOP');
  const instance=new WebAssembly.Instance(module,{atto:{
    // This import exists only for backward-compatible main(); portable
    // lifecycle exports must never execute it.
    cooperate:()=>{throw new Error('E_V3_JSPI_FORBIDDEN');}
  }});
  const api=instance.exports;
  if(!EXPORTS.every(name=>typeof api[name]==='function'))throw new TypeError('E_V3_EXPORTS');
  if(!loop.attachOwner(owner,epoch))throw new Error('E_V3_OWNER_ADMISSION');

  let started=false,finished=false,cancelRequested=false,frameLive=false;
  let pendingResolve,pendingReject,turns=0,resumes=0,cleanups=0;
  let promise;
  function cleanup(cancellation=false) {
    if(frameLive) {
      if(cancellation) {
        const status=api.task_cancel_v3(ownerId,epoch);
        if(status!==CANCELLED&&status!==INVALID)throw new Error('E_V3_CANCEL_STATUS');
      }
      const dropped=api.task_drop_v3(ownerId,epoch);
      if(dropped!==READY)throw new Error('E_V3_DROP_STATUS');
      frameLive=false;cleanups++;
    }
    finished=true;
    loop.retireOwner(owner,epoch);
  }
  function finish(error,value) {
    if(finished)return;
    try{cleanup(!!error)}catch(cleanupError){error??=cleanupError;}
    if(error)pendingReject(error);else pendingResolve(value);
  }
  function resume() {
    if(finished)return;
    turns++;
    if(cancelRequested){finish(new Error('E_V3_CANCELLED'));return;}
    try{
      const status=api.task_resume_v3(ownerId,epoch);
      resumes++;
      if(status===DONE){
        const value=api.task_result_v3(ownerId,epoch);
        finish(null,value);return;
      }
      if(status!==PENDING)throw new Error('E_V3_RESUME_STATUS');
      if(!loop.admitData(owner,epoch,resume))throw new Error('E_V3_READY_QUOTA');
    }catch(error){finish(error);}
  }
  function begin() {
    if(finished)return;
    turns++;
    if(cancelRequested){finish(new Error('E_V3_CANCELLED'));return;}
    try{
      const status=api.task_start_v3(ownerId,epoch);
      if(status!==PENDING)throw new Error('E_V3_START_STATUS');
      frameLive=true;
      if(!loop.admitData(owner,epoch,resume))throw new Error('E_V3_READY_QUOTA');
    }catch(error){finish(error);}
  }
  function run() {
    if(started||finished)throw new Error('E_V3_ALREADY_STARTED');
    started=true;
    promise=new Promise((resolve,reject)=>{pendingResolve=resolve;pendingReject=reject;});
    // Host never executes start/resume synchronously within admission.
    try{
      if(!loop.admitData(owner,epoch,begin))finish(new Error('E_V3_READY_QUOTA'));
    }catch(error){finish(error);}
    return promise;
  }
  function cancel() {
    if(!started||finished||cancelRequested)return false;
    cancelRequested=true; // fence any DATA turns already admitted
    // CONTROL preempts DATA, and performs physical Wasm frame cleanup.
    try{loop.admitControl(owner,epoch,()=>{
      if(!finished)finish(new Error('E_V3_CANCELLED'));
    });}catch{/* a queued DATA turn still performs cleanup */}
    return true;
  }
  return Object.freeze({
    version:PORTABLE_TASK_V3,run,cancel,
    get stats(){return Object.freeze({started,finished,cancelRequested,
      frameLive,turns,resumes,cleanups});}
  });
}

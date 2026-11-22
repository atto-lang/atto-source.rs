// Portable AOT v4: bounded multi-frame Wasm guest host, one instance.
// No JSPI, no guest interpreter, no runtime-generated guest functions.
// Each lifecycle call is one synchronous Worker scheduler quantum.
// Only trusted, compiler-produced guest modules may be admitted.
import {createWorkerControlLoop} from './worker-control-loop.mjs';
export const PORTABLE_TASK_V4='atto.browser.guest.v4';
const MAX_I32=0x7fffffff;
const positive=x=>Number.isSafeInteger(x)&&x>0&&x<=MAX_I32;
const OWNER=/^[A-Za-z0-9_-]{1,128}$/;
const INVALID=-1,FULL=-2,READY=0,PENDING=1,DONE=2,CANCELLED=4;
const REQUIRED=[
  'task_start_v4','task_resume_v4','task_cancel_v4',
  'task_drop_v4','task_result_v4','task_capacity_v4'
];

export function verifyPortableTaskGuestV4(module) {
  if(!(module instanceof WebAssembly.Module))throw new TypeError('E_V4_MODULE');
  const version=WebAssembly.Module.customSections(module,PORTABLE_TASK_V4);
  if(version.length!==1||new Uint8Array(version[0]).length!==1||
     new Uint8Array(version[0])[0]!==4)
    throw new TypeError('E_V4_MARKER');
  if(WebAssembly.Module.customSections(module,'atto.browser.guest.v3').length)
    throw new TypeError('E_V4_MIXED_MARKERS');
  const compatible=WebAssembly.Module.customSections(module,'atto.async');
  if(compatible.length!==1||new Uint8Array(compatible[0]).length!==1||
     new Uint8Array(compatible[0])[0]!==2)
    throw new TypeError('E_V4_COMPAT_MARKER');
  const imports=WebAssembly.Module.imports(module);
  if(imports.length!==1||imports[0].module!=='atto'||
     imports[0].name!=='cooperate'||imports[0].kind!=='function')
    throw new TypeError('E_V4_IMPORTS');
  const exports=WebAssembly.Module.exports(module);
  for(const name of REQUIRED){
    if(exports.filter(e=>e.name===name&&e.kind==='function').length!==1)
      throw new TypeError('E_V4_EXPORTS');
  }
  if(exports.some(e=>e.kind==='memory'||e.kind==='table'||e.name==='task_invoke'))
    throw new TypeError('E_V4_EXPOSED_ARENA');
  return true;
}

export function createPortableTaskArenaHostV4({
  module,loop=createWorkerControlLoop()
}={}) {
  verifyPortableTaskGuestV4(module);
  if(!loop||!['attachOwner','admitData','admitControl','retireOwner']
      .every(k=>typeof loop[k]==='function'))
    throw new TypeError('E_V4_LOOP');
  const instance=new WebAssembly.Instance(module,{atto:{
    // The v4 functions must never call the legacy JSPI main import.
    cooperate(){throw new Error('E_V4_JSPI_FORBIDDEN');}
  }});
  const api=instance.exports;
  if(!REQUIRED.every(n=>typeof api[n]==='function')||
     api.task_capacity_v4()!==16)
    throw new Error('E_V4_CAPACITY');
  const active=new Map(),capacity=16;
  let closed=false,started=0,turns=0,resumes=0,cleanups=0,peak=0,liveFrames=0;
  let errors=0;

  function settle(task,error,value) {
    if(task.finished)return;
    task.finished=true;
    let cause=error;
    try{
      if(task.handle>0){
        if(error){
          try{
            const status=api.task_cancel_v4(task.handle,task.ownerId,task.epoch);
            if(status!==CANCELLED&&status!==INVALID)
              throw new Error('E_V4_CANCEL_STATUS');
          }catch(e){cause??=e;}
        }
        try{
          const status=api.task_drop_v4(task.handle,task.ownerId,task.epoch);
          if(status!==READY)throw new Error('E_V4_DROP_STATUS');
          cleanups++;liveFrames--;
        }catch(e){
          cause??=e;
          // A guest with a failed destructor is not safe to reuse.
          closed=true;
        }
        task.handle=0;
      }
    }finally{
      active.delete(task.owner);
      loop.retireOwner(task.owner,task.epoch);
    }
    if(cause){errors++;task.reject(cause);}else task.resolve(value);
  }
  function queueResume(task) {
    if(!loop.admitData(task.owner,task.epoch,()=>resume(task)))
      throw new Error('E_V4_READY_QUOTA');
  }
  function resume(task) {
    if(task.finished)return;
    turns++;
    if(task.cancelRequested){settle(task,new Error('E_V4_CANCELLED'));return;}
    try{
      const status=api.task_resume_v4(task.handle,task.ownerId,task.epoch);
      resumes++;
      if(status===DONE){
        const value=api.task_result_v4(task.handle,task.ownerId,task.epoch);
        settle(task,null,value);
      }else if(status===PENDING){
        queueResume(task);
      }else{
        throw new Error('E_V4_RESUME_STATUS');
      }
    }catch(error){settle(task,error);}
  }
  function begin(task) {
    if(task.finished)return;
    turns++;
    if(task.cancelRequested){settle(task,new Error('E_V4_CANCELLED'));return;}
    try{
      const handle=api.task_start_v4(task.ownerId,task.epoch);
      if(handle===FULL)throw new Error('E_V4_FRAME_QUOTA');
      if(!positive(handle))throw new Error('E_V4_START_STATUS');
      task.handle=handle;
      started++;liveFrames++;peak=Math.max(peak,liveFrames);
      queueResume(task);
    }catch(error){settle(task,error);}
  }
  function cancel(task) {
    if(task.finished||task.cancelRequested)return false;
    task.cancelRequested=true;
    if(loop.stats?.closed===true){
      settle(task,new Error('E_V4_SCHEDULER_CLOSED'));
      return true;
    }
    // CONTROL preempts queued DATA. If CONTROL admission is full, the next
    // already-queued DATA turn observes cancelRequested and drops the frame.
    try{
      if(!loop.admitControl(task.owner,task.epoch,()=>{
        if(!task.finished)settle(task,new Error('E_V4_CANCELLED'));
      })&&loop.stats?.closed===true)
        settle(task,new Error('E_V4_SCHEDULER_CLOSED'));
    }catch(error){
      if(loop.stats?.closed===true)settle(task,error);
    }
    return true;
  }
  function spawn({owner,ownerId,epoch}={}) {
    if(closed)throw new Error('E_V4_HOST_CLOSED');
    if(typeof owner!=='string'||!OWNER.test(owner)||
       !positive(ownerId)||!positive(epoch))
      throw new TypeError('E_V4_IDENTITY');
    if(active.size>=capacity)throw new Error('E_V4_FRAME_QUOTA');
    if(!loop.attachOwner(owner,epoch))
      throw new Error('E_V4_OWNER_ADMISSION');
    let resolve,reject;
    const result=new Promise((a,b)=>{resolve=a;reject=b;});
    const task={owner,ownerId,epoch,handle:0,finished:false,
      cancelRequested:false,resolve,reject};
    active.set(owner,task);
    try{
      if(!loop.admitData(owner,epoch,()=>begin(task)))
        settle(task,new Error('E_V4_READY_QUOTA'));
    }catch(error){settle(task,error);}
    return Object.freeze({
      result,
      cancel:()=>cancel(task),
      get stats(){return Object.freeze({
        finished:task.finished,cancelRequested:task.cancelRequested,
        frameLive:task.handle>0
      });}
    });
  }
  // Explicit owning runtime teardown. This revokes and physically drops all
  // tasks even if a caller already closed its scheduler (which discards turns).
  function close() {
    if(closed)return false;
    closed=true;
    for(const task of [...active.values()])
      settle(task,new Error('E_V4_HOST_CLOSED'));
    return true;
  }
  return Object.freeze({
    version:PORTABLE_TASK_V4,capacity,spawn,close,
    get stats(){return Object.freeze({
      closed,live:active.size,liveFrames,started,turns,resumes,cleanups,peak,errors
    });}
  });
}

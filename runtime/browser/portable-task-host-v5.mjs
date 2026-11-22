// Full-AOT portable-v5 externally pending signed-i64 futures.
// One guest Wasm instance, bounded 16-frame arena, one owner and one
// independently authenticated single-use Worker completion per pending await.
// A completion message may only enqueue a guest continuation; it cannot run
// guest code in a MessagePort or reactor handler.
import {createWorkerControlLoop} from './worker-control-loop.mjs';
import {createWorkerCompletionCapabilitiesV1} from './worker-completion-capability-v1.mjs';

export const PORTABLE_TASK_V5='atto.browser.guest.v5';
const POSITIVE_I32=x=>Number.isSafeInteger(x)&&x>0&&x<=0x7fffffff;
const OWNER=/^[A-Za-z0-9_-]{1,128}$/;
const COMPLETE=0,INVALID=-1,FULL=-2,PENDING=1,DONE=2,CANCELLED=4;
const FUNCTIONS=['task_start_v5','task_resume_v5','task_cancel_v5',
  'task_drop_v5','task_result_v5','task_capacity_v5','task_complete_v5',
  'task_pending_v5'];

export function verifyPortableTaskGuestV5(module) {
  if(!(module instanceof WebAssembly.Module))throw new TypeError('E_V5_MODULE');
  for(const [marker,expected] of [[PORTABLE_TASK_V5,5],['atto.async',2]]) {
    const parts=WebAssembly.Module.customSections(module,marker);
    if(parts.length!==1||new Uint8Array(parts[0]).length!==1||
       new Uint8Array(parts[0])[0]!==expected)
      throw new TypeError('E_V5_MARKER');
  }
  for(const prior of ['atto.browser.guest.v3','atto.browser.guest.v4'])
    if(WebAssembly.Module.customSections(module,prior).length)
      throw new TypeError('E_V5_MIXED_MARKER');
  const imports=WebAssembly.Module.imports(module);
  if(imports.length!==1||imports[0].module!=='atto'||
     imports[0].name!=='cooperate'||imports[0].kind!=='function')
    throw new TypeError('E_V5_IMPORTS');
  const exports=WebAssembly.Module.exports(module);
  for(const name of FUNCTIONS)
    if(exports.filter(x=>x.name===name&&x.kind==='function').length!==1)
      throw new TypeError('E_V5_EXPORTS');
  if(exports.some(x=>x.kind==='memory'||x.kind==='table'||x.name==='task_invoke'))
    throw new TypeError('E_V5_PRIVATE_MEMORY');
  // A custom marker is not a cryptographic code signature. The caller MUST
  // only admit trusted compiler artifacts or isolate the whole Worker.
  return true;
}

export function createPortableTaskHostV5({
  module,loop=createWorkerControlLoop()
}={}) {
  verifyPortableTaskGuestV5(module);
  if(!loop||!['beginSuspension','admitData','admitControl','retireOwner',
      'attachOwner','complete','cancel'].every(k=>typeof loop[k]==='function'))
    throw new TypeError('E_V5_LOOP');
  const api=new WebAssembly.Instance(module,{atto:{
    cooperate(){throw Error('E_V5_JSPI_FORBIDDEN');}
  }}).exports;
  if(!FUNCTIONS.every(k=>typeof api[k]==='function')||api.task_capacity_v5()!==16)
    throw new TypeError('E_V5_CAPACITY');
  const active=new Map();
  let closed=false,turns=0,starts=0,resumes=0,completions=0;
  let drops=0,liveFrames=0,peak=0,errors=0;
  const capacity=16;

  function settle(task,error,value) {
    if(task.finished)return;
    task.finished=true;
    let cause=error;
    // Retire bearer capabilities BEFORE recycling any Wasm frame. A queued
    // completion can no longer authorize a new incarnation of the same slot.
    try{task.capabilities.close();}catch(e){cause??=e;}
    try {
      if(task.handle>0) {
        if(error) {
          try{
            const status=api.task_cancel_v5(task.handle,task.ownerId,task.epoch);
            if(status!==CANCELLED&&status!==INVALID)
              throw new Error('E_V5_CANCEL_STATUS');
          }catch(e){cause??=e;}
        }
        try{
          if(api.task_drop_v5(task.handle,task.ownerId,task.epoch)!==0)
            throw Error('E_V5_DROP_STATUS');
          drops++;liveFrames--;
        }catch(e){
          cause??=e;
          closed=true; // unsafe to reuse a guest with failed frame destruction
        }
        task.handle=0;
      }
    }finally{
      task.grant=null;task.pendingKey=null;
      active.delete(task.owner);
    }
    if(cause){errors++;task.reject(cause);}else task.resolve(value);
  }

  function pending(task) {
    if(task.finished||task.cancelRequested)throw Error('E_V5_CANCELLED');
    const key=api.task_pending_v5(task.handle,task.ownerId,task.epoch);
    if(typeof key!=='bigint')throw TypeError('E_V5_KEY_TYPE');
    // Register first, publish the token only after the resume callback is
    // safely reserved. Callback is a synchronous scheduler turn, not async.
    const grant=task.capabilities.reserve(value=>completeTurn(task,key,value));
    if(!grant)throw Error('E_V5_SUSPENSION_QUOTA');
    task.grant=grant;task.pendingKey=key;
    try{
      task.onPending(Object.freeze({
        owner:task.owner,ownerId:task.ownerId,epoch:task.epoch,key,
        grant
      }));
    }catch(e){
      task.capabilities.cancel(grant);task.grant=null;task.pendingKey=null;
      throw e;
    }
  }
  function completeTurn(task,key,value) {
    if(task.finished||task.cancelRequested)return;
    turns++;
    try{
      if(task.pendingKey!==key||!task.grant)
        throw Error('E_V5_STALE_COMPLETION');
      task.grant=null;task.pendingKey=null;
      const status=api.task_complete_v5(task.handle,task.ownerId,task.epoch,key,value);
      if(status!==COMPLETE)throw Error('E_V5_GUEST_COMPLETION');
      completions++;
      if(!loop.admitData(task.owner,task.epoch,()=>resume(task)))
        throw Error('E_V5_RESUME_QUOTA');
    }catch(e){settle(task,e);}
  }
  function resume(task) {
    if(task.finished)return;
    turns++;
    if(task.cancelRequested){settle(task,Error('E_V5_CANCELLED'));return;}
    try{
      const status=api.task_resume_v5(task.handle,task.ownerId,task.epoch);
      resumes++;
      if(status===DONE) {
        const value=api.task_result_v5(task.handle,task.ownerId,task.epoch);
        settle(task,null,value);
      }else if(status===PENDING) {
        pending(task); // No guest DATA turn until its NEXT external future resolves.
      }else throw Error('E_V5_RESUME_STATUS');
    }catch(e){settle(task,e);}
  }
  function begin(task) {
    if(task.finished)return;
    turns++;
    if(task.cancelRequested){settle(task,Error('E_V5_CANCELLED'));return;}
    try{
      const handle=api.task_start_v5(task.ownerId,task.epoch);
      if(handle===FULL)throw Error('E_V5_ARENA_QUOTA');
      if(!POSITIVE_I32(handle))throw Error('E_V5_START_STATUS');
      task.handle=handle;starts++;liveFrames++;
      peak=Math.max(peak,liveFrames);
      pending(task); // This may stay pending for arbitrary real-world time.
    }catch(e){settle(task,e);}
  }
  function cancel(task) {
    if(task.finished||task.cancelRequested)return false;
    task.cancelRequested=true;
    if(task.grant){
      // Revoke the bearer *synchronously*, before CONTROL gets scheduled:
      // a racing external message can no longer claim the ticket.
      task.capabilities.cancel(task.grant);
      task.grant=null;task.pendingKey=null;
    }
    // Revocation must not wait behind an unbounded stalled completion.
    // Queued CONTROL callbacks run before DATA; if the scheduler is closed
    // or its reserved CONTROL quota is exhausted, direct destroy is safe:
    // no source-level continuation is executed during physical cleanup.
    try{
      if(!loop.admitControl(task.owner,task.epoch,()=>{
        settle(task,Error('E_V5_CANCELLED'));
      }))settle(task,Error('E_V5_CANCELLED'));
    }catch{settle(task,Error('E_V5_CANCELLED'));}
    return true;
  }
  function spawn({owner,ownerId,epoch,onPending,port}={}) {
    if(closed)throw Error('E_V5_HOST_CLOSED');
    if(typeof owner!=='string'||!OWNER.test(owner)||
       !POSITIVE_I32(ownerId)||!POSITIVE_I32(epoch)||
       typeof onPending!=='function')throw TypeError('E_V5_IDENTITY');
    if(active.has(owner)||active.size>=capacity)
      throw Error('E_V5_FRAME_QUOTA');
    if([...active.values()].some(task=>task.ownerId===ownerId))
      throw Error('E_V5_OWNER_ID_COLLISION');
    const capabilities=createWorkerCompletionCapabilitiesV1({
      loop,owner,epoch,maxPending:1
    }); // attaches this owner to the scheduler with its own epoch fence
    if(port!==undefined) {
      try{capabilities.bindPort(port);}catch(e){capabilities.close();throw e;}
    }
    let resolve,reject;
    const result=new Promise((res,rej)=>{resolve=res;reject=rej;});
    const task={owner,ownerId,epoch,onPending,capabilities,handle:0,
      grant:null,pendingKey:null,finished:false,cancelRequested:false,
      resolve,reject};
    active.set(owner,task);
    try{
      if(!loop.admitData(owner,epoch,()=>begin(task)))
        settle(task,Error('E_V5_READY_QUOTA'));
    }catch(e){settle(task,e);}
    return Object.freeze({
      result,
      cancel:()=>cancel(task),
      // For dedicated MessagePort use bindPort; this method is useful for a
      // trusted host receiving structured-clone messages and for deny tests.
      complete:frame=>capabilities.complete(frame),
      get stats(){return Object.freeze({
        finished:task.finished,frameLive:task.handle>0,
        cancelled:task.cancelRequested,pending:task.grant!==null,
        key:task.pendingKey
      });}
    });
  }
  function close() {
    if(closed)return false;
    closed=true;
    for(const task of [...active.values()])
      settle(task,Error('E_V5_HOST_CLOSED'));
    return true;
  }
  return Object.freeze({
    version:PORTABLE_TASK_V5,capacity,spawn,close,
    get stats(){return Object.freeze({
      closed,live:active.size,liveFrames,peak,turns,starts,resumes,
      completions,drops,errors
    });}
  });
}

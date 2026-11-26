// ActorFuture<int> v1: actor-incarnation-bound, single-reply, full-AOT host.
// Built on verified portable-v5 guest frames and single-use bearer completion.
// A future belongs to exactly one producer actor incarnation and task scope.
// Streams are deliberately NOT ActorFuture: ActorStream<T> needs a separate
// backpressured multi-event protocol, not repeated terminal completions.
import {createPortableTaskHostV5,verifyPortableTaskGuestV5}
  from './portable-task-host-v5.mjs';

export const ACTOR_FUTURE_REQUEST_V1='atto.actor.future.request.v1';
export const ACTOR_FUTURE_MARKER_V1='atto.browser.source.actor_future.v1';
const I32=n=>Number.isSafeInteger(n)&&n>0&&n<=0x7fffffff;
const ACTOR_IDS_MAX=8;
function manifest(module) {
  verifyPortableTaskGuestV5(module);
  const sections=WebAssembly.Module.customSections(module,ACTOR_FUTURE_MARKER_V1);
  if(sections.length!==1)throw new TypeError('E_ACTOR_FUTURE_MARKER');
  if(WebAssembly.Module.customSections(module,'atto.browser.source.future.v1').length)
    throw new TypeError('E_ACTOR_FUTURE_MIXED_MARKER');
  const buffer=sections[0],view=new DataView(buffer);
  if(buffer.byteLength<2||view.getUint8(0)!==1)throw new TypeError('E_ACTOR_FUTURE_MANIFEST');
  const count=view.getUint8(1);
  if(count<1||count>ACTOR_IDS_MAX||buffer.byteLength!==2+12*count)
    throw new TypeError('E_ACTOR_FUTURE_MANIFEST_LENGTH');
  const entries=Array.from({length:count},(_,i)=>{
    const offset=2+i*12;
    const actorId=view.getUint32(offset,true),key=view.getBigInt64(offset+4,true);
    if(!I32(actorId)||key<0n)throw new TypeError('E_ACTOR_FUTURE_MANIFEST_VALUE');
    return Object.freeze({actorId,key});
  });
  if(entries.some(entry=>entry.actorId!==entries[0].actorId))
    throw new TypeError('E_ACTOR_FUTURE_MULTI_ACTOR_UNSUPPORTED');
  return Object.freeze(entries);
}

export class ActorFutureFailure extends Error {
  constructor({actorId,incarnation,kind,reason}) {
    super('E_ACTOR_FUTURE_'+kind.toUpperCase()+': '+reason);
    this.name='ActorFutureFailure';
    this.code='E_ACTOR_FUTURE_'+kind.toUpperCase();
    this.actorId=actorId;
    this.incarnation=incarnation;
    this.kind=kind;
  }
}
export function verifyActorFutureGuestV1(module) {return manifest(module).length>0;}

export function createActorFutureHostV1({
  module,loop,onSupervisorNotice=()=>{},
  makeChannel=()=>new MessageChannel()
}={}) {
  const awaits=manifest(module);
  if(typeof onSupervisorNotice!=='function'||typeof makeChannel!=='function')
    throw new TypeError('E_ACTOR_FUTURE_CALLBACK');
  const guest=createPortableTaskHostV5({module,...(loop?{loop}:{})});
  const actors=new Map(),lastIncarnation=new Map(),tasks=new Set();
  let closed=false,requests=0,revocations=0,notices=0;

  function registerActor({actorId,incarnation,controlPort}) {
    if(closed)throw Error('E_ACTOR_FUTURE_HOST_CLOSED');
    if(!I32(actorId)||!I32(incarnation)||
       !controlPort||typeof controlPort.postMessage!=='function')
      throw new TypeError('E_ACTOR_FUTURE_ACTOR_IDENTITY');
    // This manifest admits exactly one producer identity. Do not retain
    // unbounded historical IDs for actors this guest cannot request.
    if(actorId!==awaits[0].actorId)throw Error('E_ACTOR_FUTURE_UNDECLARED_ACTOR');
    if(actors.has(actorId))throw Error('E_ACTOR_FUTURE_ACTOR_ALREADY_REGISTERED');
    if(incarnation<=(lastIncarnation.get(actorId)||0))
      throw Error('E_ACTOR_FUTURE_STALE_INCARNATION');
    const entry={actorId,incarnation,controlPort};
    actors.set(actorId,entry);
    lastIncarnation.set(actorId,incarnation);
    return true;
  }
  function endTask(task) {
    tasks.delete(task);
    try{task.port1?.close();}catch{}
    try{task.port2?.close();}catch{}
    task.port1=null;task.port2=null;
  }
  function spawn({owner,ownerId,epoch}={}) {
    if(closed)throw Error('E_ACTOR_FUTURE_HOST_CLOSED');
    const actorId=awaits[0].actorId,actor=actors.get(actorId);
    if(!actor)throw Error('E_ACTOR_FUTURE_ACTOR_NOT_REGISTERED');
    const channel=makeChannel(),port1=channel?.port1,port2=channel?.port2;
    if(!port1||!port2||typeof port1.addEventListener!=='function'||
       typeof port2.postMessage!=='function')throw new TypeError('E_ACTOR_FUTURE_REPLY_PORT');
    const task={actorId,incarnation:actor.incarnation,owner,step:0,
      port1,port2,transferred:false,failed:null,controller:null};
    tasks.add(task);
    try{
      const controller=guest.spawn({owner,ownerId,epoch,port:port1,
        onPending:({key,grant})=>{
          if(task.failed)throw task.failed;
          const current=actors.get(actorId);
          if(!current||current.incarnation!==task.incarnation)
            throw new ActorFutureFailure({actorId,incarnation:task.incarnation,
              kind:'terminated',reason:'producer incarnation retired'});
          const entry=awaits[task.step];
          if(!entry||entry.actorId!==actorId||entry.key!==key)
            throw Error('E_ACTOR_FUTURE_PENDING_KEY_MISMATCH');
          // This closure is synchronous; no awaited host callbacks run here.
          // The actor obtains ONLY its own per-task reply port and grant.
          const message={
            type:ACTOR_FUTURE_REQUEST_V1,actorId,
            incarnation:task.incarnation,requestIndex:task.step,
            key,owner,epoch,grant
          };
          if(!task.transferred) {
            message.port=task.port2;
            current.controlPort.postMessage(message,[task.port2]);
            task.transferred=true;
          }else{
            current.controlPort.postMessage(message);
          }
          task.step++;requests++;
        }});
      task.controller=controller;
      const result=controller.result.then(
        value=>{endTask(task);return value;},
        error=>{endTask(task);throw task.failed??error;}
      );
      return Object.freeze({
        result,
        cancel:()=>controller.cancel(),
        get stats(){return Object.freeze({
          actorId,incarnation:task.incarnation,
          requestIndex:task.step,...controller.stats
        });}
      });
    }catch(error){endTask(task);throw error;}
  }
  function stopActor({actorId,incarnation,kind='terminated',reason='actor stopped'}={}) {
    if(!I32(actorId)||!I32(incarnation)||
       !['terminated','panic','raise'].includes(kind)||
       typeof reason!=='string'||reason.length>256)
      throw new TypeError('E_ACTOR_FUTURE_TERMINATION');
    const actor=actors.get(actorId);
    if(!actor||actor.incarnation!==incarnation)return false;
    actors.delete(actorId); // fence admission and all future requests first
    const failure=new ActorFutureFailure({actorId,incarnation,kind,reason});
    for(const task of tasks){
      if(task.actorId===actorId&&task.incarnation===incarnation) {
        task.failed=failure;
        if(task.controller?.cancel()){revocations++;}
      }
    }
    // This is the trusted host -> supervisor CONTROL signal. Never send
    // panic to the faulting actor for interception. The host must wire this
    // callback to its supervisor, not to untrusted Worker message handling.
    notices++;
    try{
      const observed=onSupervisorNotice(Object.freeze({actorId,incarnation,kind,reason}));
      if(observed&&typeof observed.then==='function')
        Promise.resolve(observed).catch(()=>{});
    }catch{ /* a failing notice sink must not undo revocation */ }
    return true;
  }
  function close() {
    if(closed)return false;
    closed=true;
    for(const actor of [...actors.values()])
      stopActor({actorId:actor.actorId,incarnation:actor.incarnation,
        kind:'terminated',reason:'host closed'});
    guest.close();
    for(const task of [...tasks])endTask(task);
    return true;
  }
  return Object.freeze({
    version:ACTOR_FUTURE_REQUEST_V1,awaits,registerActor,stopActor,spawn,close,
    get stats(){return Object.freeze({
      closed,activeActors:actors.size,liveTasks:tasks.size,
      requests,revocations,notices,...guest.stats
    });}
  });
}

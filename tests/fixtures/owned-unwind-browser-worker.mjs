import {instantiateOwnedUnwind,startOwnedUnwind} from '../../runtime/browser/owned-unwind-host-v1.mjs';
import {createWorkerControlLoop} from '/js/src/worker-control-loop.mjs';
import {SelectableI64Future} from '/js/src/selectable-i64-future.mjs';
import * as runtime from '/js/src/scheduled-unwind-runtime.mjs';
const bytes=await(await fetch('/guest.wasm')).arrayBuffer();
const send=kind=>self.postMessage({kind});
const err=Error('original'),futures=Array.from({length:3},()=>new SelectableI64Future()),[input,cleanup,fallback]=futures;
const check=(v,m)=>{if(!v)throw Error(m);};
const report=e=>self.postMessage({kind:'error',message:String(e)});
const scheduler=createWorkerControlLoop({maxSuspended:1,onTurnError:report});scheduler.attachOwner('a',1);
const after=fn=>setTimeout(()=>{try{fn();}catch(e){report(e);}},0);
let phase=0;
check(startOwnedUnwind({instance:instantiateOwnedUnwind(new WebAssembly.Module(bytes)),scheduler,runtime,ownerKey:'a',owner:7,epoch:1,futures,onDone:v=>{
 check(phase===3&&v.kind==='recovered'&&v.value===42n&&v.error===err,'result or inline wake');check(!scheduler.retireOwner('a',1),'lease released during publication');
 phase=4;self.postMessage({kind:'done',value:String(v.value),sameError:v.error===err});
}}),'admission');
after(()=>{check(input.pendingWaiters===1&&!scheduler.retireOwner('a',1),'body not parked');send('body');});
self.addEventListener('message',event=>{const m=event.data;try{
 if(m==='fail'){check(phase===0,'phase');phase=1;input.fail(err);check(cleanup.pendingWaiters===0,'inline');after(()=>{check(cleanup.pendingWaiters===1,'cleanup');send('cleanup');});}
 if(m==='clean'){check(phase===1,'phase');phase=2;cleanup.resolve(0n);check(fallback.pendingWaiters===0,'inline');after(()=>{check(fallback.pendingWaiters===1,'recover');send('recover');});}
 if(m==='fallback'){check(phase===2,'phase');phase=3;fallback.resolve(42n);check(phase===3,'inline');}
 if(m==='retire'){check(phase===4&&scheduler.stats.retainedOwners===0&&scheduler.retireOwner('a',1)&&scheduler.attachOwner('a',2),'retirement');send('retired');}
}catch(e){report(e);}});

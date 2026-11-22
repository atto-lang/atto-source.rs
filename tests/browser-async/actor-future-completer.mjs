// Nested Chromium module Worker acting as one trusted actor incarnation.
// Single-reply ActorFuture<int> uses a per-task MessagePort. On a streamed
// second message the capability is already spent: the host rejects it.
const replies=new Map(),held=[];
let control,released=false;
function complete(m) {
  if(m.type!=='atto.actor.future.request.v1'||m.actorId!==7||
     m.incarnation!==1||![41n,42n].includes(m.key))
    throw Error('E_ACTOR_FUTURE_METADATA');
  const port=replies.get(m.owner);
  if(!port)throw Error('E_ACTOR_FUTURE_REPLY_PORT');
  port.postMessage({...m.grant,value:m.key===41n?111n:42n});
  port.postMessage({...m.grant,value:777n}); // stale/replayed reply denied
}
self.onmessage=({data})=>{
  if(data.kind==='bind'){
    control=data.port;
    control.onmessage=({data:m})=>{
      if(m.port)replies.set(m.owner,m.port);
      if(released)complete(m);else held.push(m);
    };
    control.start();
    self.postMessage({kind:'ready'});
  }
  if(data.kind==='release'){
    released=true;
    for(const m of held.splice(0))complete(m);
  }
};

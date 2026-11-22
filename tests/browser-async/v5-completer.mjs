// Real Chromium nested-module-Worker completion issuer. The holder of a
// single-use capability may complete ONLY its own host-admitted future.
const ports=new Map(),held=[];
let released=false;
function fulfill(message) {
  const port=ports.get(message.owner);
  if(!port)throw Error('E_V5_MISSING_PORT');
  port.postMessage({...message.grant,
    value:message.key===41n?111n:42n});
}
self.onmessage=({data})=>{
  if(data.kind==='bind') {
    ports.set(data.owner,data.port);
    return;
  }
  if(data.kind==='grant') {
    if(released)fulfill(data);
    else held.push(data);
    return;
  }
  if(data.kind==='release') {
    released=true;
    for(const message of held.splice(0))fulfill(message);
  }
};

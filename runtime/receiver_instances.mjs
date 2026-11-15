// Restricted actor-instance host for the opt-in receiver Wasm preview.
// The caller MUST provide an authoritative tenant context; a caller-supplied
// tenant string is not authentication. No scheduler, mailbox, shared state,
// borrow checking, or production supervisor is implemented here.
const MAX_I64=(1n<<63n)-1n, MIN_I64=-(1n<<63n);
const NAME=/^[A-Za-z_][A-Za-z_0-9]*$/;

function secure_token() {
  if (!globalThis.crypto?.getRandomValues)
    throw new Error('secure WebCrypto randomness is required for actor capabilities');
  const words=new Uint32Array(2);
  do {globalThis.crypto.getRandomValues(words);}
  while(words[0]===0 && words[1]===0);
  return BigInt.asIntN(64,(BigInt(words[0])<<32n)|BigInt(words[1]));
}

export function create_receiver_runtime(wasm_bytes,{on_control=()=>{}}={}) {
  if (typeof on_control!=='function') throw new TypeError('on_control must be a function');
  const wasm_module=new WebAssembly.Module(wasm_bytes);
  if (WebAssembly.Module.imports(wasm_module).length!==0)
    throw new Error('instance receiver preview cannot import host functions');
  const refs=new WeakMap();
  let next_id=0;

  function validate_tenant(tenant) {
    if (typeof tenant!=='string' || tenant.length===0)
      throw new TypeError('tenant context must be a nonempty trusted identifier');
  }
  function get_record(tenant,handle) {
    validate_tenant(tenant);
    if (handle===null || typeof handle!=='object')
      throw new TypeError('invalid actor capability');
    const rec=refs.get(handle);
    if (!rec) throw new TypeError('forged actor capability');
    if (rec.tenant!==tenant) throw new Error('cross-tenant actor capability denied');
    return rec;
  }
  function close_record(rec) {
    if (rec.closed) return false;
    // Mark terminal before invoking any callback or user-visible operation.
    rec.closed=true;
    if (rec.exports['atto.actor_close'](rec.token)!==1)
      throw new Error('internal actor close rejected');
    return true;
  }
  function spawn_actor(tenant,owner) {
    validate_tenant(tenant);
    if (typeof owner!=='string' || !NAME.test(owner))
      throw new TypeError('invalid nominal actor type');
    const instance=new WebAssembly.Instance(wasm_module,{});
    const exports=instance.exports;
    const bind=exports['atto.bind.'+owner];
    if (typeof bind!=='function') throw new Error('unknown actor type: '+owner);
    const token=secure_token();
    if (bind(token)!==1) throw new Error('actor instance bind failed');
    const handle=Object.freeze(Object.create(null));
    const rec={id:++next_id,generation:1,tenant,owner,exports,token,closed:false};
    refs.set(handle,rec);
    return handle;
  }
  function invoke_method(tenant,handle,method,...args) {
    const rec=get_record(tenant,handle);
    if (rec.closed) throw new Error('actor instance is closed');
    if (typeof method!=='string' || !NAME.test(method))
      throw new TypeError('invalid receiver method name');
    if (args.length>8 || args.some(v=>typeof v!=='bigint'||v<MIN_I64||v>MAX_I64))
      throw new TypeError('receiver arguments must be signed i64 BigInt values');
    const fn=rec.exports['atto.receiver.'+rec.owner+'.'+method+'.'+args.length];
    if (typeof fn!=='function') throw new Error('unknown receiver method/arity');
    try {
      return fn(rec.token,...args);
    } catch(err) {
      if (!(err instanceof WebAssembly.RuntimeError)) throw err;
      const code=rec.exports.panic_code.value;
      close_record(rec);
      // This is a synchronous host notification hook, not the production
      // actor supervisor's priority CONTROL queue.
      const notice=Object.freeze({
        kind:'PANIC',actor_id:rec.id,generation:rec.generation,
        actor_type:rec.owner,tenant:rec.tenant,panic_code:code
      });
      try {on_control(notice);} catch(_notification_error) {
        // Never hide the original guest trap or resurrect the actor.
      }
      throw err;
    }
  }
  function close_actor(tenant,handle) {
    return close_record(get_record(tenant,handle));
  }
  return Object.freeze({spawn_actor,invoke_method,close_actor});
}

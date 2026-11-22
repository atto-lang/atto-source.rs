// Bounded monotonic browser Worker timer adapter, not an Atto stdlib.
// The guest's await still yields through JSPI; this is its trusted host edge.
// No guest code or actor continuation executes from the timer callback.
export class MonotonicSleepProvider {
  constructor({
    now=()=>performance.now(),
    schedule=(callback, delay)=>setTimeout(callback, delay),
    unschedule=handle=>clearTimeout(handle),
    maxPending=64
  }={}) {
    if(!Number.isSafeInteger(maxPending)||maxPending<1||maxPending>4096)
      throw new RangeError('invalid timer admission capacity');
    this.now=now;
    this.schedule=schedule;
    this.unschedule=unschedule;
    this.maxPending=maxPending;
    this.pending=0;
  }

  sleep(ms, signal=undefined) {
    if(!Number.isSafeInteger(ms)||ms<0||ms>1000)
      throw new RangeError('invalid or unsupported JSPI preview sleep duration');
    if(signal?.aborted)
      return Promise.reject(new Error('timer cancelled'));
    if(this.pending>=this.maxPending)
      throw new RangeError('JSPI timer registration budget exhausted');

    // Capturing once prevents wall-clock jumps, resettable deadlines and
    // cumulative rounding drift on spurious/early platform wakeups.
    const started=this.now();
    if(!Number.isFinite(started)||started<0)
      throw new Error('invalid monotonic clock reading');
    let lastObserved=started;
    const deadline=started+ms;
    if(!Number.isFinite(deadline))
      throw new Error('monotonic deadline overflow');
    this.pending++;

    return new Promise((resolve,reject)=>{
      let handle=null,finished=false;
      const settle=error=>{
        if(finished)return;
        finished=true;
        if(handle!==null)this.unschedule(handle);
        signal?.removeEventListener('abort',onAbort);
        this.pending--;
        if(error)reject(error);
        else resolve();
      };
      const onAbort=()=>settle(new Error('timer cancelled'));
      const arm=delay=>{
        try{handle=this.schedule(check,delay)}
        catch(error){settle(error)}
      };
      const check=()=>{
        handle=null;
        if(signal?.aborted){onAbort();return}
        let observed;
        try{observed=this.now()}
        catch(error){settle(error);return}
        // Do not silently lengthen a timeout when the supposed monotonic
        // provider resets, regresses or produces a nonfinite/negative tick.
        // The real Worker uses performance.now(), which must be monotonic.
        if(!Number.isFinite(observed)||observed<lastObserved){
          settle(new Error('invalid or regressed monotonic clock reading'));
          return;
        }
        lastObserved=observed;
        const remaining=deadline-observed;
        if(!Number.isFinite(remaining)){
          settle(new Error('invalid monotonic deadline computation'));
          return;
        }
        if(remaining<=0){settle();return}
        arm(Math.max(1,Math.ceil(remaining)));
      };
      signal?.addEventListener('abort',onAbort,{once:true});
      if(signal?.aborted){onAbort();return}
      // Even 0ms runs on a subsequent Worker event-loop turn.
      arm(Math.max(0,Math.ceil(ms)));
    });
  }
}

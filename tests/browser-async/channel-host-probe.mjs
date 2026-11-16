// Real Chromium module Worker execution of the typed Channel<int> host v1.
// Deliberately NOT a compiler-generated Atto guest channel read.
import {ChannelIntHostV1} from '/channel-int-v1.mjs';

self.onmessage=async()=>{
  try{
    const rt=new ChannelIntHostV1({maxChannels:2,maxTickets:4,maxCapacity:2});
    const created=rt.open(1);
    if(!created.ok)throw Error('channel open failed');
    const ch=created.value;
    const pending=rt.nb_read(ch);
    if(!pending.ok)throw Error('pending read failed');
    const received=rt.await_read(pending.value);
    // A native Worker task delivers after registering a nonparking Future.
    await new Promise(resolve=>setTimeout(resolve,0));
    if(!rt.send(ch,41n).ok)throw Error('send failed');
    const item=await received;
    if(!item.ok||!item.value.some||item.value.value!==41n)
      throw Error('pending read did not resume with 41');
    const double=await rt.await_read(pending.value);
    if(double.ok||double.error.kind!=='AlreadyConsumed')
      throw Error('Future was consumed twice');

    const abandoned=rt.nb_read(ch).value;
    if(!rt.cancel(abandoned).ok)throw Error('cancel did not retire pending read');
    const cancelled=await rt.await_read(abandoned);
    if(cancelled.ok||cancelled.error.kind!=='Cancelled')
      throw Error('cancel result lost its error');

    const awaiting=rt.nb_read(ch).value;
    if(!rt.close(ch).ok)throw Error('close failed');
    const eof=await rt.await_read(awaiting);
    if(!eof.ok||eof.value.some)throw Error('EOF became value');
    const closed=rt.send(ch,1n);
    if(closed.ok||closed.error.kind!=='Closed')throw Error('send-after-close admitted');

    self.postMessage({
      ok:true,answer:String(item.value.value+1n),
      singleConsumption:double.error.kind,
      cancellation:cancelled.error.kind,
      eof:!eof.value.some,sendAfterClose:closed.error.kind
    });
  }catch(e){self.postMessage({ok:false,error:String(e?.message||e)})}
};

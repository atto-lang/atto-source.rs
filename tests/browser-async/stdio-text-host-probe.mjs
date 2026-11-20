// Real Chromium Worker proof of the injected host stdio text adapter.
// Not a compiled .atto guest / not a Wasm Channel<string> import.
import {StdioTextHostV1} from '/runtime/browser/stdio_text_host_v1.mjs';

self.onmessage=async()=>{
  try {
    async function* injectedInput() {
      yield new TextEncoder().encode('alpha\r\nβ\0z\n');
    }
    const output=[],error=[];
    const port=new StdioTextHostV1({
      generation:7,tenantId:'chromium-stdio-probe',
      stdin:injectedInput(),
      stdout:async bytes=>{output.push(new TextDecoder().decode(bytes))},
      stderr:async bytes=>{error.push(new TextDecoder().decode(bytes))},
    });
    if(port.stdio.in!==port.process.stdin || port.stdio.out!==port.process.stdout ||
       port.stdio.err!==port.process.stderr) throw Error('aliases are not identical');
    const first=await port.process.stdin.readch();
    if(!first.ok||!first.value.some||first.value.value!=='alpha')
      throw Error('first injected record was corrupted');
    const stream=port.stdio.in.stream();
    const competing=await port.stdio.in.readch();
    if(competing.ok||competing.error.kind!=='ReaderBusy')
      throw Error('stream did not exclusively claim stdin');
    const second=await stream.next();
    if(second.done||second.value!=='β\0z')throw Error('Unicode/NUL lost');
    const eof=await stream.next();
    if(!eof.done)throw Error('EOF became a message');
    if(!(await port.stdio.out.writech('Echo:')).ok ||
       !(await port.process.stdout.writech(first.value.value)).ok ||
       !(await port.stdio.err.writech('echo complete')).ok)
      throw Error('checked output admission failed');
    port.revoke();
    const denied=await port.stdio.in.readch();
    if(denied.ok||denied.error.kind!=='Revoked')throw Error('revocation bypassed');
    self.postMessage({ok:true,alias:true,first:first.value.value,second:second.value,
      eof:true,readerConflict:competing.error.kind,stdout:output.join(''),
      stderr:error.join(''),postRevoke:denied.error.kind});
  } catch(e) { self.postMessage({ok:false,error:String(e?.stack||e)}) }
};

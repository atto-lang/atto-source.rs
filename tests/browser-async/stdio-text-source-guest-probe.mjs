// Browser Worker: real changed-Rust-compiler Wasm -> .atto source -> guest
// Wasm -> authenticated checked JSPI imports. NOT a hand-written Wasm guest.
import {StdioTextHostV1} from '/runtime/browser/stdio_text_host_v1.mjs';
import {createStdioTextGuestV1,verifyStdioTextGuestV1}
  from '/runtime/browser/stdio_text_guest_v1.mjs';

const echoSource="pub suspends fnc main(): int using io, suspend {\n  val input = stdio.in;\n  val output = process.stdout;\n  val prompt = writech output, \"Echo the next input item:\";\n  return match prompt over\n    on Err(_) -> 10,\n    on Ok(_) -> match << input over\n      on Err(_) -> 14,\n      on Ok(line) -> match line over\n        on None -> 11,\n        on Some(item) -> match writech output, item over\n          on Err(_) -> 12,\n          on Ok(_) -> match writech stdio.err, \"echo complete\" over\n            on Ok(_) -> 0,\n            on Err(_) -> 13,\n          end,\n        end,\n      end,\n    end,\n  end;\n}";
const encoder=new TextEncoder(), decoder=new TextDecoder();
const pause=()=>new Promise(resolve=>setTimeout(resolve,0));
self.onmessage=async()=>{
  try {
    if(typeof WebAssembly.Suspending!=='function'||
       typeof WebAssembly.promising!=='function')
      throw Error('JSPI unavailable');
    const compilerBytes=await (await fetch('/atto_compiler.wasm')).arrayBuffer();
    if(!WebAssembly.validate(compilerBytes))throw Error('compiler Wasm invalid');
    const compiler=new WebAssembly.Instance(
      new WebAssembly.Module(compilerBytes),{}).exports;
    function compile(source) {
      const encoded=encoder.encode(source);
      if(encoded.length>compiler.input_capacity())throw Error('fixture source too long');
      new Uint8Array(compiler.memory.buffer,compiler.input_ptr(),encoded.length).set(encoded);
      if(compiler.compile(encoded.length)!==0) {
        const data=new Uint8Array(compiler.memory.buffer);
        let end=compiler.error_ptr();
        while(end<data.length&&data[end]&&end-compiler.error_ptr()<512)end++;
        throw Error('source compiler failed: '+decoder.decode(
          data.subarray(compiler.error_ptr(),end)));
      }
      const bytes=new Uint8Array(compiler.memory.buffer,
        compiler.result_ptr(),compiler.result_length()).slice();
      if(!WebAssembly.validate(bytes))throw Error('compiler generated invalid guest Wasm');
      const mod=new WebAssembly.Module(bytes);
      if(!verifyStdioTextGuestV1(bytes,mod))
        throw Error('compiled echo lacks authenticated checked stdio marker');
      return mod;
    }
    const isSource=echoSource
      .replaceAll('on Err(_)', 'on is Err<error>')
      .replaceAll('on Ok(_)', 'on is Ok<_>')
      .replaceAll('on Ok(line)', 'on is Ok<line>')
      .replaceAll('on Some(item)', 'on is Some<item>')
      .replaceAll('on None', 'on is None');
    if(!isSource.includes('on is Ok<_> -> match << input over') ||
       !isSource.includes('on is Ok<line> -> match line over') ||
       !isSource.includes('on is Some<item>'))
      throw Error('nominal is-ADT fixture rewrite lost the requested nested match');
    const variants=[
      ['parenthesized+unary',echoSource],
      ['parenthesized+readch',echoSource.replace('match << input over','match readch input over')],
      ['on-is+unary',isSource],
      ['on-is+readch',isSource.replace('match << input over','match readch input over')],
    ];
    const compiledVariants=variants.map(([label,source])=>[label,compile(source)]);
    const importsExpected=JSON.stringify(WebAssembly.Module.imports(compiledVariants[0][1]));
    for(const [name,guest] of compiledVariants) {
      if(JSON.stringify(WebAssembly.Module.imports(guest))!==importsExpected)
        throw Error('read/ADT variants disagree on checked guest import signatures: '+name);
    }
    async function run(label,inputParts,expected,grantOverride={},guestModule=compiledVariants[0][1]) {
      const stdout=[],stderr=[];
      const stdin=(async function*() {for(const part of inputParts)yield part})();
      const host=new StdioTextHostV1({
        tenantId:'echo-worker-tenant',generation:101,
        stdin,stdout:async bytes=>stdout.push(decoder.decode(bytes)),
        stderr:async bytes=>stderr.push(decoder.decode(bytes)),
      });
      let instance,cooperates=0;
      const grant={tenantId:'echo-worker-tenant',generation:101,
        allowStdin:true,allowStdout:true,allowStderr:true,...grantOverride};
      const adapter=createStdioTextGuestV1({
        getMemory:()=>instance.exports.memory,host,grant});
      const imports={...adapter.imports,
        stdio_text_read_v1:new WebAssembly.Suspending(adapter.imports.stdio_text_read_v1),
        stdio_text_write_v1:new WebAssembly.Suspending(adapter.imports.stdio_text_write_v1),
        cooperate:new WebAssembly.Suspending(async()=>{await pause();cooperates++}),
      };
      instance=new WebAssembly.Instance(guestModule,{atto:imports});
      const exit=Number(await WebAssembly.promising(instance.exports.main)());
      const observed={exit,stdout:stdout.join(''),stderr:stderr.join('')};
      for(const [key,value] of Object.entries(expected))
        if(observed[key]!==value)throw Error(label+' '+key+': expected '+JSON.stringify(value)+
          ' got '+JSON.stringify(observed[key]));
      if(!cooperates)throw Error(label+' compiled guest did not cooperate');
      adapter.revoke();
      return {label,exit,cooperates};
    }
    const cases=[];
    const prompt='Echo the next input item:';
    cases.push(await run('LF echo',[encoder.encode('hello\n')],
      {exit:0,stdout:prompt+'hello',stderr:'echo complete'}));
    cases.push(await run('CRLF + NUL',[encoder.encode('β\0x\r\n')],
      {exit:0,stdout:prompt+'β\0x',stderr:'echo complete'}));
    cases.push(await run('empty record',[encoder.encode('\n')],
      {exit:0,stdout:prompt,stderr:'echo complete'}));
    cases.push(await run('unterminated final',[encoder.encode('last')],
      {exit:0,stdout:prompt+'last',stderr:'echo complete'}));
    cases.push(await run('clean EOF',[],
      {exit:11,stdout:prompt,stderr:''}));
    cases.push(await run('bad UTF-8',[new Uint8Array([0xc3,0x28,10])],
      {exit:14,stdout:prompt,stderr:''}));
    cases.push(await run('stdin denied',[encoder.encode('secret\n')],
      {exit:14,stdout:prompt,stderr:''},{allowStdin:false}));
    cases.push(await run('stdout denied',[encoder.encode('unused\n')],
      {exit:10,stdout:'',stderr:''},{allowStdout:false}));
    cases.push(await run('stderr denied',[encoder.encode('hello\n')],
      {exit:13,stdout:prompt+'hello',stderr:''},{allowStderr:false}));
    // All four source combinations must execute inside the actual guest
    // runtime: old/new ADT constructors x unary/explicit checked read.
    // Host-only adapter tests are insufficient for on-is pattern parity.
    const proofCases=[
      ['LF echo',[encoder.encode('hello\n')],{exit:0,stdout:prompt+'hello',stderr:'echo complete'},{}],
      ['CRLF + NUL',[encoder.encode('β\0x\r\n')],{exit:0,stdout:prompt+'β\0x',stderr:'echo complete'},{}],
      ['empty record',[encoder.encode('\n')],{exit:0,stdout:prompt,stderr:'echo complete'},{}],
      ['unterminated final',[encoder.encode('last')],{exit:0,stdout:prompt+'last',stderr:'echo complete'},{}],
      ['clean EOF',[],{exit:11,stdout:prompt,stderr:''},{}],
      ['bad UTF-8',[new Uint8Array([0xc3,0x28,10])],{exit:14,stdout:prompt,stderr:''},{}],
      ['stdin denied',[encoder.encode('secret\n')],{exit:14,stdout:prompt,stderr:''},{allowStdin:false}],
      ['stdout denied',[encoder.encode('unused\n')],{exit:10,stdout:'',stderr:''},{allowStdout:false}],
      ['stderr denied',[encoder.encode('hello\n')],{exit:13,stdout:prompt+'hello',stderr:''},{allowStderr:false}],
    ];
    const patternExecutions=[];
    for(const [name,guest] of compiledVariants) {
      for(const [caseName,parts,expected,grants] of proofCases) {
        const result=await run(name+' '+caseName,parts,expected,grants,guest);
        patternExecutions.push({pattern:name,case:caseName,exit:result.exit});
      }
    }
    // Execute the *string-returning* nested match requested by the user;
    // main is necessarily int/void in this compiler, so a suspends wrapper
    // awaits the string-producing callable and performs a checked write.
    const fallbackSource=`
pub suspends fnc next_text(): string using io, suspend {
  val input = stdio.in;
  val output = stdio.out;
  val prompt = writech output, "prompt";
  return match prompt over
    on is Ok<_> -> match << input over
      on is Ok<line> -> match line over
        on is Some<item> -> item,
        on None -> "",
      end,
      on is Err<error> -> "",
    end,
    on is Err<error> -> "",
  end;
}
pub suspends fnc main(): int using io, suspend {
  val item = await next_text();
  val receipt = writech stdio.out, item;
  return match receipt over
    on is Ok<_> -> 0,
    on is Err<error> -> 12,
  end;
}
`;
    const fallbackModule=compile(fallbackSource);
    const fallbackCases=[];
    for(const [name,parts,expected,grant] of [
      ['LF value',[encoder.encode('hello\n')],{exit:0,stdout:'prompthello',stderr:''},{}],
      ['None EOF',[],{exit:0,stdout:'prompt',stderr:''},{}],
      ['Err UTF-8',[new Uint8Array([0xc3,0x28,10])],{exit:0,stdout:'prompt',stderr:''},{}],
      ['Err denied stdin',[encoder.encode('secret\n')],{exit:0,stdout:'prompt',stderr:''},{allowStdin:false}],
      ['Err denied stdout',[encoder.encode('hello\n')],{exit:12,stdout:'',stderr:''},{allowStdout:false}],
    ]) {
      const result=await run('exact-string-fallback '+name,parts,expected,grant,fallbackModule);
      fallbackCases.push({name,exit:result.exit});
    }
    self.postMessage({ok:true,compiledAtto:true,checkedStdio:true,
      directReadchAbiEquivalent:true,nominalIsAdtGuest:true,
      patternVariants:compiledVariants.map(([name])=>name),
      patternExecutions,stringFallbackGuest:true,fallbackCases,cases});
  } catch(e) {
    self.postMessage({ok:false,error:String(e?.stack||e)});
  }
};

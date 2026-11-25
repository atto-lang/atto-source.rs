import assert from 'node:assert/strict';
import {spawnSync} from 'node:child_process';
import {existsSync,mkdtempSync,readFileSync,rmSync,writeFileSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {join,resolve} from 'node:path';
const cli=resolve(process.env.ATTOC || 'target/release/attoc');
const dir=mkdtempSync(join(tmpdir(),'atto-durable-actor-'));
const source=`define durable actor Counter as
    state count: int = 0;
    receive(ActorMail<int> mail) -> void {
        self.count = self.count + mail.value;
    }
end
`;
function compile(text,mode,positive) {
    const input=join(dir,'actor.atto'), output=join(dir,'actor.wasm');
    writeFileSync(input,text);rmSync(output,{force:true});
    const result=spawnSync(cli,[...(mode?['--durable-counter-actor-v1']:[]),input,'-o',output],
        {cwd:dir,encoding:'utf8',timeout:30000,maxBuffer:1<<20});
    assert.equal(result.error,undefined);
    if(!positive) {
        assert.equal(result.status,1,result.stderr);
        assert.ok(result.stderr.trim());
        assert.equal(existsSync(output),false);
        return;
    }
    assert.equal(result.status,0,result.stderr);
    const bytes=readFileSync(output);
    assert.ok(WebAssembly.validate(bytes));
    const mod=new WebAssembly.Module(bytes);
    assert.deepEqual(WebAssembly.Module.imports(mod),[]);
    assert.deepEqual(WebAssembly.Module.exports(mod).map(x=>x.name).sort(),
        ['atto_durable_transition_v1','main','panic_code']);
    return new WebAssembly.Instance(mod,{}).exports;
}
try {
    const guest=compile(source,true,true);
    assert.equal(guest.main(),0n);
    const t=guest.atto_durable_transition_v1;
    assert.equal(t(0n,1n,42n),42n);
    assert.equal(t(42n,1n,5n),47n);
    assert.equal(t(47n,2n,0n),47n);
    assert.equal(t(999999999n,1n,1n),1000000000n);
    assert.equal(t(1000000000n,1n,1n),-1n);
    assert.equal(t(0n,1n,-1n),-1n);
    assert.equal(t(-1n,1n,1n),-1n);
    assert.equal(t(0n,3n,1n),-1n);
    assert.equal(guest.panic_code.value,0);
    const renamed=compile(source.replaceAll('count','balance').replaceAll('mail','event'),true,true);
    assert.equal(renamed.atto_durable_transition_v1(1n,1n,2n),3n);
    for(const invalid of [
        source.replace('durable actor','actor'),
        source.replace('durable actor','persistent actor'),
        source.replace('= 0;','= 1;'),
        source.replace('state count','pub state count'),
        source.replace('self.count + mail.value','self.count - mail.value'),
        source.replace('mail.value','9'),
        source.replace('ActorMail<int>','ActorMail<bool>'),
        source.replace('self.count =','self.unknown ='),
        source.replace('receive(','pub receive('),
        source.replace('self.count = self.count + mail.value;','self.count = self.count + mail.value; self.count = 0;'),
        source+'\npub fnc main()->int {return 0;}',
    ]) compile(invalid,true,false);
    // The regular compiler must still fail closed on durable actors.
    compile(source+'\npub fnc main()->int {return 0;}',false,false);
    console.log('COMPILED_DURABLE_COUNTER_ACTOR_ABI_PASS');
} finally {rmSync(dir,{recursive:true,force:true});}

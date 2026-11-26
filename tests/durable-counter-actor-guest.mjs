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
// Independently decode the generated section and recompute the canonical
// wire checksum and descriptor fingerprint. Never trust a guest's self-report.
function fnv(data) {
    let h=0xcbf29ce484222325n;
    for(const b of data) h=BigInt.asUintN(64,(h ^ BigInt(b))*0x100000001b3n);
    return h;
}
function sized(bytes) {
    const len=Buffer.alloc(4);len.writeUInt32LE(bytes.length);
    return Buffer.concat([len,bytes]);
}
function verifySchema(mod,field) {
    const sections=WebAssembly.Module.customSections(mod,'atto.durable.state.v1');
    assert.equal(sections.length,1);
    const buf=Buffer.from(sections[0]);let i=0;
    assert.equal(buf[i++],1);
    function take() {
        let len=0,shift=0,b;
        do {
            assert.ok(i<buf.length && shift<=28,'invalid metadata length');
            b=buf[i++];len+=(b&127)*2**shift;shift+=7;
        } while(b&128);
        assert.ok(len<=buf.length-i,'metadata length outside section');
        const out=buf.subarray(i,i+len);i+=len;return out;
    }
    const actor=take(),name=take();
    assert.equal(actor.toString('utf8'),'Counter');
    assert.equal(name.toString('utf8'),field);
    assert.equal(buf[i++],1);
    const initial=take();assert.equal(i,buf.length);
    assert.equal(initial.length,36);
    assert.equal(initial.subarray(0,8).toString('ascii'),'ATTOST01');
    assert.equal(initial[8],1);
    assert.equal(initial.readBigUInt64LE(9),
        fnv(Buffer.concat([Buffer.from('atto.actor.state.schema.v1\\0'.replace('\\0','\0')),sized(actor),sized(name),Buffer.from([1])])));
    assert.equal(initial.readUInt16LE(17),1);
    assert.equal(initial[19],1);
    assert.equal(initial.readBigInt64LE(20),0n);
    assert.equal(initial.readBigUInt64LE(28),fnv(initial.subarray(0,28)));
    return Buffer.from(sections[0]);
}
function compile(text,mode,positive,field='count') {
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
    const descriptor=verifySchema(mod,field);
    return {guest:new WebAssembly.Instance(mod,{}).exports,bytes,descriptor};
}
try {
    const first=compile(source,true,true);
    const guest=first.guest;
    const repeat=compile(source,true,true);
    assert.deepEqual(first.bytes,repeat.bytes,'deterministic durable artifact');
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
    const renamed=compile(source.replaceAll('count','balance').replaceAll('mail','event'),true,true,'balance');
    assert.equal(renamed.guest.atto_durable_transition_v1(1n,1n,2n),3n);
    assert.notDeepEqual(renamed.descriptor,first.descriptor,'renamed state has distinct schema');
    assert.notDeepEqual(renamed.bytes,first.bytes,'artifact binds state identity');
    const mailRenamed=compile(source.replaceAll('mail','event'),true,true);
    assert.deepEqual(mailRenamed.descriptor,first.descriptor,'mail parameter is not state schema');
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

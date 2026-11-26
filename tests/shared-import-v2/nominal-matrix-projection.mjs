// Project exactly the shared JSON fixture into a dependency-free Rust test transport.
// The test runner exercises the production resolver; this script is not an oracle.
import {readFileSync} from 'node:fs';
const doc=JSON.parse(readFileSync(new URL('./fixtures/import-v2/cases.json',import.meta.url),'utf8'));
const fixture=doc.fixtures['nominal-kind-matrix'];
const cases=doc.cases.filter(c=>c.id.startsWith('nominal-matrix-'));
if(cases.length!==35||fixture.identities.length!==2)throw Error('matrix incomplete');
const lines=[];
for(const [i,name] of fixture.identities.entries())lines.push(['package',name,String(i+1).repeat(64)]);
for(const [name,kind,pub] of fixture.exports)lines.push(['export',name,kind,String(pub)]);
for(const c of cases){
  if(c.stage!=='resolution'||c.fixture!=='nominal-kind-matrix')throw Error(c.id);
  const e=c.expect;
  lines.push(['case',c.id,c.source.replaceAll('\n','\\n'),c.reference,e.outcome,e.identity??'',e.declaration??'',e.code??'',String(e.atomic??false)]);
}
for(const row of lines)if(row.some(v=>/[\t\r\n]/.test(v)))throw Error('ambiguous projection');
console.log(lines.map(row=>row.join('\t')).join('\n'));

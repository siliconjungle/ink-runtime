import assert from 'node:assert/strict';
import {readFile,mkdtemp,writeFile,rm} from 'node:fs/promises';
import {tmpdir} from 'node:os';
import {pathToFileURL} from 'node:url';
const dir=await mkdtemp(tmpdir()+'/ink-credits-');
try{
 const route={schema:1,semantics:'ink-literal-source-routing-v1',input_core_sha256:'a'.repeat(64),entry:'entry',stages:[{id:'a',function:'first',backend:'c',domain:'cpu',arguments:[{Input:0}]},{id:'b',function:'second',backend:'c',domain:'cpu',arguments:[{Stage:'a'}]}],output:{Stage:'b'}};
 const plan={releases:[{after:'b',values:['a']}],admission:{max_pending:2,max_pending_bytes:32}};
 await writeFile(dir+'/ink-route.mjs',`const ROUTE=${JSON.stringify(route)};const RESOURCES=${JSON.stringify(plan)};\n`+await readFile(new URL('../hosts/ink-route.mjs',import.meta.url),'utf8'));
 await writeFile(dir+'/ink-gpu.mjs',`export async function loadInk(){return {manifest:{core_sha256:'${'a'.repeat(64)}',functions:[{name:'entry',params:[{type:'list_u32'}],result:'u32'},{name:'first',params:[{type:'list_u32'}],result:'u32'},{name:'second',params:[{type:'u32'}],result:'u32'}]},capture(n,a){return [Uint32Array.from(a[0])]},async run(n,a){await new Promise(r=>setTimeout(r,2));if(n==='first'&&a[0][0]===13)throw Error('failure');return {value:n==='first'?a[0][0]+1:a[0]*2,backend:'cpu'}},async close(){}}}`);
 const events=[];const engine=await (await import(pathToFileURL(dir+'/ink-route.mjs'))).loadRoute(dir,{onUsage:e=>events.push(e)});
 const input=new Uint32Array([4]);const first=engine.run([input]);input[0]=100;
 const second=engine.run([new Uint32Array([8])]);await assert.rejects(engine.run([input]),e=>e.code==='INK_BACKPRESSURE');
 assert.equal((await first).value,10);assert.equal((await second).value,18);
 await assert.rejects(engine.run([new Uint32Array(9)]),e=>e.code==='INK_BACKPRESSURE');
 await assert.rejects(engine.run([new Uint32Array([13])]),/failure/);
 assert.equal((await engine.run([new Uint32Array([2])])).value,6);
 assert.equal(events.filter(e=>e.event==='completed'&&e.phase==='call:entry').length,3);assert.equal(events.filter(e=>e.event==='failed').length,1);
 await engine.close();await assert.rejects(engine.run([input]),/closed/);
 console.log('compiled graph resources: capture, ordered queue, byte/count overload, failure recovery and closure passed');
}finally{await rm(dir,{recursive:true,force:true});}

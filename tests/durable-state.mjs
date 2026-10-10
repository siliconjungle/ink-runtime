import test from 'node:test';
import assert from 'node:assert/strict';
import {mkdtemp,readFile,writeFile,rm} from 'node:fs/promises';
import {tmpdir} from 'node:os';
import {join} from 'node:path';
import {DurableState,RecoveryRequired,recordToken} from '../hosts/durable-state.mjs';
import {openFileStore} from '../hosts/durable-file.mjs';
import {parseExactJSON,stringifyExactJSON} from '../hosts/state-wasm.mjs';
const encoder=new TextEncoder(),decoder=new TextDecoder();
const factory={
  create(){return state({version:0n,value:0n,outbox:[]});},
  restore(bytes){return state(parseExactJSON(decoder.decode(bytes)));},
};
function state(s){return {
  invoke(call,args){if(call==='fail')throw Error('action failed');if(call==='read')return {value:s.value,host:Number(s.value),zero:-0};s.value+=BigInt(args[0]);s.version++;s.outbox.push({commit:s.version,position:0n,channel:'changed',value:s.value});return {value:s.value,version:s.version};},
  checkpoint:()=>encoder.encode(stringifyExactJSON(s)),
  get version(){return s.version;},get outbox(){return s.outbox;},
  acknowledgeThrough(c,p){s.outbox=s.outbox.filter(e=>e.commit>c||(e.commit===c&&e.position>p));},
};}
function memory(){let saved=null;return {load:async()=>saved&&{token:saved.token,bytes:saved.bytes.slice()},commit:async(expected,bytes)=>{if((saved?.token??null)!==expected)throw Error('conflict');saved={token:await recordToken(bytes),bytes:bytes.slice()};return saved.token;},close(){}};}
test('capture, serialize concurrent calls and recover exact receipts',async()=>{
 const store=memory();let host=await DurableState.open({factory,store});const args=[2];const call=host.invoke('a','add',args);args[0]=99;
 assert.equal((await call).value,2n);const pending=Array.from({length:12},(_,i)=>host.invoke('id'+i,'add',[1]));await Promise.all(pending);
 assert.equal(await host.version(),13n);assert.equal((await host.invoke('r','read',[])).host,14);const saved=await host.checkpoint();await host.close();
 host=await DurableState.open({factory,store});assert.equal((await host.invoke('a','add',[2])).value,2n);assert.equal(await host.version(),13n);
 const reply=await host.invoke('r','read',[]);assert.equal(typeof reply.host,'number');assert(Object.is(reply.zero,-0));assert.deepEqual(await host.checkpoint(),saved);
 await assert.rejects(host.invoke('a','add',[3]),/reused/);await host.close();
});
test('uncertain write before or after durable publication requires recovery',async()=>{
 for(const after of [false,true]){
  const base=memory();let armed=false;const store={...base,async commit(t,b){if(!armed)return base.commit(t,b);if(after)await base.commit(t,b);throw Error('simulated power loss');}};
  let host=await DurableState.open({factory,store});armed=true;
  await assert.rejects(host.invoke('change','add',[7]),RecoveryRequired);await assert.rejects(host.version(),RecoveryRequired);await host.close();
  host=await DurableState.open({factory,store:base});assert.equal(await host.version(),after?1n:0n);
  assert.equal((await host.invoke('change','add',[7])).value,7n);assert.equal(await host.version(),1n);await host.close();
 }
});
test('receiver failure leaves pending events; acknowledgement is durable',async()=>{
 const store=memory();let host=await DurableState.open({factory,store});await host.invoke('a','add',[1]);await host.invoke('b','add',[2]);const seen=[];
 await assert.rejects(host.deliver(async(e,key)=>{seen.push(key);if(key==='2:0')throw Error('receiver unavailable');}),/unavailable/);
 assert.deepEqual(seen,['1:0','2:0']);await host.close();host=await DurableState.open({factory,store});assert.equal((await host.events()).length,1);
 assert.equal(await host.deliver(async(e,key)=>{assert.equal(key,'2:0');}),1);await host.close();host=await DurableState.open({factory,store});assert.deepEqual(await host.events(),[]);await host.close();
});
test('delivery can repeat after receiver success and failed acknowledgement',async()=>{
 const base=memory();let armed=false;const store={...base,commit:async(t,b)=>{if(armed)throw Error('write failure');return base.commit(t,b);}};
 let host=await DurableState.open({factory,store});await host.invoke('a','add',[1]);armed=true;const seen=new Set();let calls=0;
 const receive=async(_,key)=>{seen.add(key);calls++;};await assert.rejects(host.deliver(receive),RecoveryRequired);await host.close();
 host=await DurableState.open({factory,store:base});await host.deliver(receive);assert.equal(calls,2);assert.equal(seen.size,1);await host.close();
});
test('action exception rolls back; bounded receipt eviction is explicit',async()=>{
 const store=memory();const host=await DurableState.open({factory,store,receiptLimit:1});const before=await host.checkpoint();await assert.rejects(host.invoke('bad','fail',[]));assert.deepEqual(await host.checkpoint(),before);
 await host.invoke('a','add',[1]);await host.invoke('b','read',[]);await host.invoke('a','add',[1]);assert.equal(await host.version(),2n);await host.close();
});
test('storage conflict rejects a second stale writer',async()=>{
 const store=memory(),a=await DurableState.open({factory,store}),b=await DurableState.open({factory,store});await a.invoke('a','add',[1]);await assert.rejects(b.invoke('b','add',[2]),RecoveryRequired);await a.close();await b.close();
});
test('real file replacement, ownership, recovery and corruption rejection',async()=>{
 const dir=await mkdtemp(join(tmpdir(),'ink-durable-'));const path=join(dir,'state');
 try{
  let host=await DurableState.open({factory,store:await openFileStore(path)});await assert.rejects(openFileStore(path),/writer/);
  await host.invoke('a','add',[18446744073709551615n]);await host.close();host=await DurableState.open({factory,store:await openFileStore(path)});
  assert.equal((await host.invoke('a','add',[18446744073709551615n])).value,18446744073709551615n);await host.close();
  const bytes=await readFile(path);bytes[bytes.length-40]^=1;await writeFile(path,bytes);
  await assert.rejects(DurableState.open({factory,store:await openFileStore(path)}),/checksum/);
  const store=await openFileStore(path);await store.close();
 }finally{await rm(dir,{recursive:true,force:true});}
});

import {copySharedHostAssets} from './shared-host-assets.mjs';
import test from 'node:test';
import assert from 'node:assert/strict';
import {mkdtemp,copyFile,writeFile,rm} from 'node:fs/promises';
import {tmpdir} from 'node:os';
import {join} from 'node:path';
import {pathToFileURL} from 'node:url';
const types=['u32','u64','f32'].map(kind=>({kind}));
const functions=types.map((element,index)=>({index,name:element.kind,params:[{type:{kind:'list',element}}],result:{kind:'list',element},shader:null}));
async function fixture(run) {
 const dir=await mkdtemp(join(tmpdir(),'ink-destination28-'));
 await copySharedHostAssets(dir);
 for(const name of ['ink-compute.mjs','backend-selection.mjs','prepared-wasm.mjs','complete-session.mjs','pipeline-contract.mjs'])await copyFile(new URL(`../hosts/${name}`,import.meta.url),join(dir,name));
 await writeFile(join(dir,'ink-device.mjs'),'export const dispatchCount=p=>p.steps.length;export const createDevice=()=>({close(){}});');
 await writeFile(join(dir,'program.mjs'),`const copy=(xs,out)=>{for(let i=0;i<xs.length;i++)out[i]=xs[i];return xs.length};export const metadata={functions:[]};export const functions={u32:xs=>Array.from(xs),u64:xs=>Array.from(xs),f32:xs=>Array.from(xs)};export const functionsInto={u32:copy,u64:copy,f32:copy};`);
 const originalFetch=globalThis.fetch,originalInstantiate=WebAssembly.instantiate;
 let loads=0;
 globalThis.fetch=async url=>String(url).endsWith('manifest.json')?{json:async()=>({schema:2,functions})}:{arrayBuffer:async()=>new ArrayBuffer(0)};
 WebAssembly.instantiate=async(...args)=>{if(args[0]?.byteLength)return originalInstantiate(...args);
  loads++;let top=64;
  const memory=new WebAssembly.Memory({initial:1});
  return {instance:{exports:{memory,ink_compute_reset(){top=64},ink_alloc(n,width){let p=(top+7)&~7;top=p+Math.max(n,1)*width;return p},ink_compute_mark(){return top},ink_compute_rewind(mark){top=mark},ink_compute_into(index,pointers,lengths,dest){const table=new DataView(memory.buffer),p=table.getUint32(pointers,true),count=table.getUint32(lengths,true)*(index===1?2:1);new Uint32Array(memory.buffer,dest,count).set(new Uint32Array(memory.buffer,p,count));return count}}}};
 };
 try {const {loadInk}=await import(pathToFileURL(join(dir,'ink-compute.mjs')));const runtime=await loadInk(pathToFileURL(dir+'/'),{gpu:false});await run(runtime,()=>loads);await runtime.close();}
 finally{globalThis.fetch=originalFetch;WebAssembly.instantiate=originalInstantiate;await rm(dir,{recursive:true,force:true});}
}
test('native JS destination preserves U64/F32 native values and untouched tail without Wasm loading',async()=>fixture(async(runtime,loads)=>{
 const out=new BigUint64Array([0n,0n,33n]);
 assert.equal((await runtime.callInto('u64',[[18446744073709551615n,1n]],out)).count,2);
 assert.deepEqual(out,new BigUint64Array([18446744073709551615n,1n,33n]));
 const floats=[-0,{F32Bits:2143289344}],dest=[0,0,'tail'];
 assert.equal((await runtime.callInto('f32',[floats],dest)).count,2);assert.ok(Object.is(dest[0],-0));assert.deepEqual(dest[1],floats[1]);assert.equal(dest[2],'tail');
 assert.equal(loads(),0);
}));
test('captured CPU destination returns native BigInt and exact floating bits, with stable ordinary results',async()=>fixture(async runtime=>{
 const out=new BigUint64Array(3);out[2]=77n;
 await runtime.callInto('u64',[[18446744073709551615n,9007199254740992n]],out,{backend:'cpu'});
 assert.deepEqual(out,new BigUint64Array([18446744073709551615n,9007199254740992n,77n]));
 const dest=[0,0,'tail'];await runtime.callInto('f32',[[-0,{F32Bits:2143289344}]],dest,{backend:'cpu'});
 assert.ok(Object.is(dest[0],-0));assert.deepEqual(dest[1],{F32Bits:2143289344});assert.equal(dest[2],'tail');
 const owned=(await runtime.call('u32',[[4,5]],{backend:'javascript'})).value;
 await runtime.callInto('u32',[[8,9]],new Uint32Array(2));assert.deepEqual(owned,[4,5]);
}));
test('prepared CPU calls snapshot once and remain independent of ordinary calls',async()=>fixture(async(runtime,loads)=>{
 const input=Uint32Array.of(1,2),prepared=await runtime.prepareCpuCall('u32',[input],4);input.fill(99);
 assert.deepEqual(Array.from(prepared.run()),[1,2]);
 await runtime.callInto('u32',[[7,8]],new Uint32Array(2),{backend:'cpu'});
 assert.deepEqual(Array.from(prepared.run()),[1,2]);
 prepared.writeArgument(0,Uint32Array.of(11),1);assert.deepEqual(Array.from(prepared.run()),[11]);
 assert.equal(loads(),2,'prepared and ordinary execution have separate instances');prepared.close();
}));

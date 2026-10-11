import {copySharedHostAssets} from './shared-host-assets.mjs';
import test from 'node:test';
import assert from 'node:assert/strict';
import {mkdtemp,copyFile,writeFile,rm} from 'node:fs/promises';
import {tmpdir} from 'node:os';
import {join} from 'node:path';
import {pathToFileURL} from 'node:url';

const types=['u32','i32','bool','f32','u64'].map(kind=>({kind}));
const functions=types.map((type,index)=>({index,name:type.kind,params:[{type}],result:type,shader:null}));
functions.push({index:5,name:'vector',params:[{type:{kind:'list',element:types[0]}}],result:{kind:'list',element:types[0]},shader:null});

// This exercises transport/ownership independently of C expression emission.
// The backend suite separately compares the actual C dispatchers to Ink's
// interpreter; compiled Wasm bundles are validated by the integration run.
async function fixture(direct,run) {
 const dir=await mkdtemp(join(tmpdir(),'ink-scalar-bridge25-'));
 await copySharedHostAssets(dir);
 for(const name of ['ink-compute.mjs','backend-selection.mjs','prepared-wasm.mjs'])await copyFile(new URL(`../hosts/${name}`,import.meta.url),join(dir,name));
 await writeFile(join(dir,'ink-device.mjs'),"export const createDevice=()=>({close(){},prepare(){throw Error('GPU disabled')},run(){throw Error('GPU disabled')}});");
 await writeFile(join(dir,'program.mjs'),"export const metadata={functions:[]};export const functions={u32:x=>x,i32:x=>x,bool:x=>x,f32:x=>x,u64:x=>x,vector:x=>x};");
 const oldFetch=globalThis.fetch,oldInstantiate=WebAssembly.instantiate;
 const memory=new WebAssembly.Memory({initial:2});let top=64,failNext=false;
 const counts={allocations:[],generic:0,word32:0,word64:0,resets:0};
 const ptr=(table,index=0)=>new DataView(memory.buffer).getUint32(table+index*4,true);
 const words=(table,index)=>new Uint32Array(memory.buffer,ptr(table),index===4?2:1);
 const exports={memory,ink_compute_reset(){top=64;counts.resets++},ink_alloc(count,width){const result=top;top+=count*width;counts.allocations.push([count,width]);return result;},ink_compute_call(index,table,lengths,header,countPointer){
  counts.generic++;const input=ptr(table),n=index===5?ptr(lengths):index===4?2:1;
  const output=exports.ink_alloc(n,4);new Uint32Array(memory.buffer,output,n).set(new Uint32Array(memory.buffer,input,n));
  const view=new DataView(memory.buffer);view.setUint32(header,output,true);view.setUint32(countPointer,n,true);
 }};
 if(direct){
  exports.ink_compute_scalar32=(index,table)=>{counts.word32++;if(failNext){failNext=false;throw Error('synthetic target failure')}assert.ok(index<4);return words(table,index)[0]|0;};
  exports.ink_compute_scalar64=(index,table)=>{counts.word64++;assert.equal(index,4);const a=words(table,index);return BigInt.asIntN(64,BigInt(a[0])|(BigInt(a[1])<<32n));};
 }
 globalThis.fetch=async url=>String(url).endsWith('manifest.json')?{json:async()=>({schema:2,core_sha256:'transport-fixture25',functions})}:{arrayBuffer:async()=>new ArrayBuffer(0)};
 WebAssembly.instantiate=async(...args)=>args[0]?.byteLength?oldInstantiate(...args):({instance:{exports}});
 try{
  const {loadInk}=await import(pathToFileURL(join(dir,'ink-compute.mjs')));
  const runtime=await loadInk(pathToFileURL(`${dir}/`),{gpu:false,backend:'cpu'});
  await run(runtime,counts,()=>{failNext=true});await runtime.close();
 }finally{globalThis.fetch=oldFetch;WebAssembly.instantiate=oldInstantiate;await rm(dir,{recursive:true,force:true});}
}

test('scalar machine-word results preserve all numeric representations without header/output allocations',async()=>fixture(true,async(runtime,counts)=>{
 const cases=[['u32',4294967295,4294967295],['i32',-2147483648,-2147483648],['bool',true,true],['bool',false,false],['f32',-0,-0],['f32',{F32Bits:0x7fc12345},{F32Bits:0x7fc12345}],['f32',{F32Bits:0xff800000},{F32Bits:0xff800000}],['u64',18446744073709551615n,'18446744073709551615'],['u64',9223372036854775808n,'9223372036854775808'],['u64',0n,0]];
 for(const [name,input,expected] of cases){
  const before=counts.allocations.length;
  const value=(await runtime.call(name,[input])).value;
  assert.deepEqual(value,expected);if(name==='f32'&&Object.is(input,-0))assert.ok(Object.is(value,-0));
  assert.equal(counts.allocations.length-before,3,'only pointer table, length table and captured argument');
 }
 assert.equal(counts.generic,0);assert.equal(counts.word32,7);assert.equal(counts.word64,3);
 assert.equal(counts.resets,20,'one initial and final reset per call');
 const xs=new Uint32Array([0,4294967295,17]),before=xs.slice();
 assert.deepEqual((await runtime.call('vector',[xs])).value,Array.from(xs));assert.deepEqual(xs,before);
 assert.equal(counts.generic,1,'list output retains generic packed ABI');
}));

test('modules without scalar exports retain the previous packed result path',async()=>fixture(false,async(runtime,counts)=>{
 assert.equal((await runtime.call('u32',[4294967295])).value,4294967295);
 assert.equal((await runtime.call('u64',[18446744073709551615n])).value,'18446744073709551615');
 assert.equal(counts.generic,2);assert.equal(counts.word32,0);assert.equal(counts.word64,0);
 assert.equal(counts.allocations.length,10,'each call retains header and result allocation');
}));

test('failed scalar calls reset the arena and leave the serialized queue usable',async()=>fixture(true,async(runtime,counts,failNext)=>{
 failNext();await assert.rejects(runtime.call('u32',[7]),/synthetic target failure/);
 assert.equal(counts.resets,2);assert.equal((await runtime.call('u32',[9])).value,9);
 assert.equal(counts.resets,4);assert.equal(counts.generic,0);
}));

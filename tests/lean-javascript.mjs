import {copySharedHostAssets} from './shared-host-assets.mjs';
import test from 'node:test';
import assert from 'node:assert/strict';
import {mkdtemp,copyFile,writeFile,rm} from 'node:fs/promises';
import {tmpdir} from 'node:os';
import {join} from 'node:path';
import {pathToFileURL} from 'node:url';

const u32={kind:'u32'},i32={kind:'i32'},u64={kind:'u64'},bool={kind:'bool'},f32={kind:'f32'};
const list=element=>({kind:'list',element});
const specs=[
 ['increment',[list(u32)],list(u32)], ['negative',[list(i32)],list(bool)],
 ['count',[list(bool)],u64], ['wide',[list(u64)],list(u64)],
 ['identity',[list(u32)],list(u32)], ['signed',[list(i32)],list(i32)],
 ['floatIdentity',[list(f32)],list(f32)],
 ['wideIdentity',[list(u64)],list(u64)], ['wideNested',[list(list(u64))],list(list(u64))],
];
const functions=specs.map(([name,params,result],index)=>({index,name,params:params.map(type=>({type})),result,shader:null}));

async function fixture(run,options={}) {
 const dir=await mkdtemp(join(tmpdir(),'ink-lean-runtime-'));
 await copySharedHostAssets(dir);
 for(const name of ['ink-compute.mjs','backend-selection.mjs','prepared-wasm.mjs','complete-session.mjs','pipeline-contract.mjs'])await copyFile(new URL(`../hosts/${name}`,import.meta.url),join(dir,name));
 await writeFile(join(dir,'ink-device.mjs'),"export const dispatchCount=p=>p.steps.length;export const createDevice=()=>({close(){},prepare(){throw Error('GPU unavailable')},run(){throw Error('GPU unavailable')},capture(p){const outputs=p.outputs.slice();return {setup:{upload_bytes:4},run(){return {values:Object.fromEntries(outputs.map(id=>[id,[7]])),counters:{upload_bytes:0}}},close(){}}}});");
 await writeFile(join(dir,'program.mjs'),`export const metadata={functions:[]};export const functions={
  increment:xs=>Array.from(xs,x=>(x+1)>>>0),negative:xs=>Array.from(xs,x=>x<0),count:xs=>BigInt(xs.length),
  wide:xs=>Array.from(xs,x=>BigInt.asUintN(64,x+1n)),identity:xs=>xs,
  signed:xs=>Array.from(xs,x=>(x+1)|0),floatIdentity:xs=>xs,wideIdentity:xs=>xs,wideNested:xs=>xs
 };`);
 const oldFetch=globalThis.fetch,oldInstantiate=WebAssembly.instantiate;
 let cpuCalls=0,wasmLoads=0;
 const memory=new WebAssembly.Memory({initial:1});let top=0;
 const exports={memory,ink_compute_reset(){top=0},ink_alloc(count,align){top=Math.ceil(top/align)*align;const ptr=top;top+=count*4;return ptr},ink_compute_call(index,ptrs,lengths,hp,hc){
  cpuCalls++;const view=new DataView(memory.buffer),ptr=view.getUint32(ptrs,true),len=view.getUint32(lengths,true),xs=Array.from(new Uint32Array(memory.buffer,ptr,len));
  assert.equal(index,0,'mock CPU only handles increment');const words=xs.map(x=>(x+1)>>>0),out=exports.ink_alloc(words.length,4);new Uint32Array(memory.buffer,out,words.length).set(words);view.setUint32(hp,out,true);view.setUint32(hc,words.length,true);
 }};
 const manifestFunctions=options.gpu?functions.map(f=>f.name==='increment'?{...f,shader:'mock.wgsl',lengths:[0]}:f):functions;
 globalThis.fetch=async url=>String(url).endsWith('manifest.json')?{json:async()=>({schema:2,core_sha256:'fixture',functions:manifestFunctions})}:{arrayBuffer:async()=>new ArrayBuffer(0)};
 WebAssembly.instantiate=async(...args)=>{if(args[0]?.byteLength)return oldInstantiate(...args);wasmLoads++;return {instance:{exports}}};
 try {
  const {loadInk}=await import(pathToFileURL(join(dir,'ink-compute.mjs')));
  const runtime=await loadInk(pathToFileURL(`${dir}/`),{gpu:false,backend:'javascript',...options});
  await run(runtime,()=>cpuCalls,()=>wasmLoads);
 }finally{globalThis.fetch=oldFetch;WebAssembly.instantiate=oldInstantiate;await rm(dir,{recursive:true,force:true});}
}

test('bound trusted JS calls preserve wrapping, signed, Bool, U64 and F32 outputs without CPU transport',async()=>fixture(async(runtime,cpuCalls,wasmLoads)=>{
 const xs=new Uint32Array([0,4294967295,2147483648]);const before=xs.slice();
 assert.deepEqual((await runtime.call('increment',[xs])).value,[1,0,2147483649]);assert.deepEqual(xs,before);
 assert.deepEqual((await runtime.call('signed',[[-2147483648,2147483647,-1]])).value,[-2147483647,-2147483648,0]);
 assert.deepEqual((await runtime.call('negative',[[-1,0,2147483647]])).value,[true,false,false]);
 assert.equal((await runtime.call('count',[[true,false]])).value,2);
 assert.deepEqual((await runtime.call('wide',[[0n,18446744073709551615n,9007199254740991n]])).value,[1,0,'9007199254740992']);
 const wideTyped=new BigUint64Array([1n,18446744073709551615n]);
 assert.deepEqual((await runtime.call('wideIdentity',[wideTyped])).value,[1,'18446744073709551615']);
 assert.deepEqual((await runtime.call('wideNested',[[wideTyped]])).value,[[1,'18446744073709551615']]);
 assert.deepEqual(wideTyped,new BigUint64Array([1n,18446744073709551615n]));
 const floats=[-0,1.5,{F32Bits:2143289344}];assert.strictEqual((await runtime.call('floatIdentity',[floats])).value,floats);
 assert.strictEqual((await runtime.call('identity',[xs])).value,xs,'native identity borrows stable caller values');
 assert.equal(cpuCalls(),0);
 assert.equal(wasmLoads(),0,'explicit JS never loads or instantiates the fallback');
 await runtime.close();await assert.rejects(runtime.call('increment',[xs]),/closed/);
}));

test('JS pipeline composes native Bool/list/U64 values and feedback, matching literal references',async()=>fixture(async runtime=>{
 const xs=[-1,0,-2147483648,2147483647];
 const r=await runtime.pipeline({steps:[{id:'flags',call:'negative',args:[xs]},{id:'count',call:'count',args:[{step:'flags'}]}],outputs:['flags','count']});
 assert.deepEqual(r.values,{flags:xs.map(x=>x<0),count:xs.length});
 const initial=new Uint32Array([4294967295,1]);
 const iterated=await runtime.pipeline({inputs:{xs:initial},steps:[{id:'next',call:'increment',args:[{input:'xs'}]}],feedback:{xs:'next'},iterations:3,outputs:['next']});
 assert.deepEqual(iterated.values.next,Array.from(initial,x=>(x+3)>>>0));assert.deepEqual(initial,new Uint32Array([4294967295,1]));
 await assert.rejects(runtime.pipeline({steps:[{id:'bad',call:'count',args:[{step:'missing'}]}]}),/earlier step/);
}));

test('packed CPU remains available and profile-free auto retains equivalent CPU results',async()=>fixture(async(runtime,cpuCalls)=>{
 const xs=[4294967295,7];
 assert.deepEqual((await runtime.call('increment',[xs],{backend:'cpu'})).value,[0,8]);
 const typed=new Uint32Array([4294967295,3]);
 typed[Symbol.iterator]=()=>{throw Error('typed ABI snapshots must not iterate each word')};
 const pending=runtime.call('increment',[typed],{backend:'cpu'});typed[0]=7;
 assert.deepEqual((await pending).value,[0,4],'packed CPU inputs snapshot once at submission');
 const noEnumeration=new Proxy(new Uint32Array([4294967295,3]),{
  ownKeys(){throw Error('transport must not enumerate typed-array elements')},
  get(target,key){const value=Reflect.get(target,key,target);return typeof value==='function'?value.bind(target):value;},
 });
 assert.deepEqual((await runtime.call('increment',[noEnumeration],{backend:'cpu'})).value,[0,4]);
 const pipeline=await runtime.pipeline({steps:[{id:'out',call:'increment',args:[noEnumeration]}],outputs:['out']},{backend:'cpu'});
 assert.deepEqual(pipeline.values.out,[0,4]);
 const r=await runtime.call('increment',[xs],{backend:'auto'});assert.deepEqual(r.value,[0,8]);assert.ok(cpuCalls()>1);
 await assert.rejects(runtime.call('increment',[xs],{backend:'invalid'}),/backend/);
}));

test('scalar host binds native JS and lazily retains its Wasm fallback',async()=>{
 const dir=await mkdtemp(join(tmpdir(),'ink-lean-scalar-'));
 await copySharedHostAssets(dir);
 for(const name of ['ink-gpu.mjs','backend-selection.mjs','pipeline-contract.mjs'])await copyFile(new URL(`../hosts/${name}`,import.meta.url),join(dir,name));
 await writeFile(join(dir,'ink-device.mjs'),"export const dispatchCount=p=>p.steps.length;export const createDevice=()=>({close(){}});");
 await writeFile(join(dir,'program.mjs'),"export const metadata={functions:[]};export const functions={advance:(x,y)=>(x+y)>>>0,total:xs=>{let n=0;for(const x of xs)n=(n+x)>>>0;return n}};");
 const manifest={schema:1,backend:'webgpu-wgpu-v1',functions:[{index:0,name:'advance',params:[{type:'u32'},{type:'u32'}],result:'u32'},{index:1,name:'total',params:[{type:'list_u32'}],result:'u32'}]};
 const oldFetch=globalThis.fetch,oldInstantiate=WebAssembly.instantiate;
 let wasmLoads=0;
 globalThis.fetch=async url=>({ok:true,json:async()=>manifest,arrayBuffer:async()=>new ArrayBuffer(0)});
 WebAssembly.instantiate=async(...args)=>{if(args[0]?.byteLength)return oldInstantiate(...args);wasmLoads++;return {instance:{exports:{__heap_base:{value:0},ink_compute_reset(){},lang_fn_advance:(x,y)=>(x+y)|0}}};};
 try {
  const {loadInk}=await import(pathToFileURL(join(dir,'ink-gpu.mjs')));
  const runtime=await loadInk(pathToFileURL(`${dir}/`),{gpu:false,backend:'javascript'});
  assert.equal((await runtime.run('advance',[4294967295,2])).value,1);
  assert.equal((await runtime.run('total',[new Uint32Array([4294967295,2])])).value,1);
  assert.equal(wasmLoads,0);
  assert.equal((await runtime.run('advance',[4294967295,2],'cpu')).value,1);
  assert.equal(wasmLoads,1);
  await runtime.close();await assert.rejects(runtime.run('advance',[1,2]),/closed/);
 }finally{globalThis.fetch=oldFetch;WebAssembly.instantiate=oldInstantiate;await rm(dir,{recursive:true,force:true});}
});

test('saved adaptive JS selection performs exactly one call and never loads Wasm',async()=>{
 const scope=JSON.stringify(['fixture',null,null,'increment']);
 const profile={schema:1,target_key:'test',models:{[scope]:{boundary:'captured-owned',domains:{input_elements:[0,100]},backends:{cpu:{intercept_ns:1000,terms:{}},javascript:{intercept_ns:10,terms:{}}}}}};
 await fixture(async(runtime,cpuCalls,wasmLoads)=>{
  for(let i=0;i<40;i++)assert.deepEqual((await runtime.call('increment',[new Uint32Array([4294967295,3])])).value,[0,4]);
  assert.equal(cpuCalls(),0);assert.equal(wasmLoads(),0);
 },{backend:'auto',policy:'adaptive',targetKey:'test',costProfile:profile});
});
test('static JS auto setting bypasses packed ingress and all profiling',async()=>{
 await fixture(async(runtime,cpuCalls,wasmLoads)=>{
  const xs=new Uint32Array([4294967295,3]);xs.slice=()=>{throw Error('static JS must not pack inputs')};
  assert.deepEqual((await runtime.call('increment',[xs])).value,[0,4]);assert.equal(cpuCalls(),0);assert.equal(wasmLoads(),0);
 },{backend:'auto',policy:'static',staticBackend:'javascript'});
});

test('prepared GPU output selection captures caller plan before queued preparation',async()=>{
 await fixture(async runtime=>{
  const outputs=['out'];const pending=runtime.prepareGpuPipeline({steps:[{id:'out',call:'increment',args:[[6]]}],outputs});
  outputs[0]='changed';outputs.length=0;const prepared=await pending;
  assert.deepEqual((await prepared.run()).value,[7]);
  outputs.push('other');assert.deepEqual((await prepared.run()).values,{out:[7]});
  await prepared.close();
 },{gpu:true});
});

test('adaptive JS reuses its private U32 capture and retains packed-host owned Array outputs',async()=>{
 const scope=JSON.stringify(['fixture',null,null,'identity']);
 const profile={schema:1,target_key:'test',models:{[scope]:{boundary:'captured-owned',domains:{input_elements:[0,100]},backends:{cpu:{intercept_ns:1000,terms:{}},javascript:{intercept_ns:10,terms:{}}}}}};
 await fixture(async(runtime,cpuCalls,wasmLoads)=>{
  const xs=Uint32Array.of(0,4294967295);const pending=runtime.call('identity',[xs]);xs.fill(99);
  const a=(await pending).value;assert.ok(Array.isArray(a));assert.deepEqual(a,[0,4294967295]);
  await runtime.call('identity',[Uint32Array.of(7)]);assert.deepEqual(a,[0,4294967295]);
  a[0]=8;assert.deepEqual((await runtime.call('identity',[Uint32Array.of(0,4294967295)])).value,[0,4294967295]);
  assert.equal(cpuCalls(),0);assert.equal(wasmLoads(),0);await runtime.close();assert.deepEqual(a,[8,4294967295]);
 },{backend:'auto',policy:'adaptive',targetKey:'test',costProfile:profile});
});

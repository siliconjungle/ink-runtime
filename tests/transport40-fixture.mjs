// Transport-only ABI fixture. Actual generated C/Rust/GPU parity is a separate
// integration gate; this deliberately poisons arena resets and grows memory.
import {mkdtemp,copyFile,writeFile,rm} from 'node:fs/promises';
import {tmpdir} from 'node:os';
import {join} from 'node:path';
import {pathToFileURL} from 'node:url';
const atom=kind=>({kind}),list=element=>({kind:'list',element});
const width=t=>t.kind==='list'?width(t.element):t.kind==='vector'?t.size*width(t.element):t.kind==='record'?t.fields.reduce((n,f)=>n+width(f.type),0):t.kind==='u64'?2:1;
export const types=[list(atom('u32')),list(atom('i32')),list(atom('bool')),list(atom('f32')),list(atom('u64')),{kind:'vector',size:2,element:atom('u64')},{kind:'record',fields:[{name:'flag',type:atom('bool')},{name:'wide',type:atom('u64')},{name:'pair',type:{kind:'vector',size:2,element:atom('f32')}}]},atom('u32')];
export const names=['words','signed','flags','floats','wide','pair','record','scalar','increment'];
export async function transportFixture(run,{host=new URL('../hosts/ink-compute.mjs',import.meta.url)}={}) {
 const dir=await mkdtemp(join(tmpdir(),'ink-transport40-'));
 await copyFile(host,join(dir,'ink-compute.mjs'));
 for(const name of ['backend-selection.mjs','prepared-wasm.mjs','pipeline-contract.mjs'])await copyFile(new URL('../hosts/'+name,import.meta.url),join(dir,name));
 await writeFile(join(dir,'ink-device.mjs'),'export const createDevice=()=>({close(){}});');
 await writeFile(join(dir,'program.mjs'),'const identity=x=>x;export const functions={words:identity,signed:identity,flags:identity,floats:identity,wide:identity,pair:identity,record:identity,scalar:identity,increment:xs=>Array.from(xs,x=>(x+1)>>>0)};export const functionsInto=Object.fromEntries(Object.entries(functions).map(([name,f])=>[name,(x,out)=>{const v=f(x);for(let i=0;i<v.length;i++)out[i]=v[i];return v.length}]));');
 const functions=names.map((name,index)=>{const t=types[index]??types[0];return {name,index,params:[{type:t}],result:t,shader:null}});
 const oldFetch=globalThis.fetch,oldInstantiate=WebAssembly.instantiate,oldSlice=Uint32Array.prototype.slice;
 const memories=new Set(),counts={arena_result_slices:0,arena_slice_words:0,reset:0,executions:0,instances:0};
 globalThis.fetch=async url=>String(url).endsWith('manifest.json')?{json:async()=>({schema:2,functions})}:{arrayBuffer:async()=>new ArrayBuffer(0)};
 WebAssembly.instantiate=async()=>{
  const memory=new WebAssembly.Memory({initial:1});memories.add(memory.buffer);counts.instances++;let top=64;
  const cpu={memory,ink_compute_reset(){new Uint32Array(memory.buffer,64,Math.floor((top-64)/4)).fill(0xdeadbeef);top=64;counts.reset++},ink_alloc(n,w){const p=(top+7)&~7;top=p+Math.max(n,1)*w;if(top>memory.buffer.byteLength){memory.grow(Math.ceil((top-memory.buffer.byteLength)/65536));memories.add(memory.buffer)}return p},ink_compute_mark(){return top},ink_compute_rewind(mark){top=mark},ink_compute_call(index,pointers,lengths,header,countPointer){
   counts.executions++;const view=new DataView(memory.buffer),ptr=view.getUint32(pointers,true),n=view.getUint32(lengths,true),t=functions[index].result,count=n*width(t),out=cpu.ink_alloc(count,4);
   // Simulate scratch allocation invalidating every pre-execution memory view.
   cpu.ink_alloc(17000,4);const a=new Uint32Array(memory.buffer,ptr,count),b=new Uint32Array(memory.buffer,out,count);for(let i=0;i<count;i++)b[i]=index===8?(a[i]+1)>>>0:a[i];
   const after=new DataView(memory.buffer);after.setUint32(header,out,true);after.setUint32(countPointer,count,true);
  },ink_compute_into(index,pointers,lengths,destination){
   counts.executions++;const view=new DataView(memory.buffer),ptr=view.getUint32(pointers,true),n=view.getUint32(lengths,true),count=n*width(functions[index].result);cpu.ink_alloc(17000,4);new Uint32Array(memory.buffer,destination,count).set(new Uint32Array(memory.buffer,ptr,count));return count;
  }};return {instance:{exports:cpu}};
 };
 Uint32Array.prototype.slice=function(...args){if(memories.has(this.buffer)){counts.arena_result_slices++;counts.arena_slice_words+=this.length}return oldSlice.apply(this,args)};
 try {const {loadInk}=await import(pathToFileURL(join(dir,'ink-compute.mjs')));const runtime=await loadInk(pathToFileURL(dir+'/'),{gpu:false,backend:'cpu'});await run(runtime,counts);await runtime.close();}
 finally {globalThis.fetch=oldFetch;WebAssembly.instantiate=oldInstantiate;Uint32Array.prototype.slice=oldSlice;await rm(dir,{recursive:true,force:true});}
}

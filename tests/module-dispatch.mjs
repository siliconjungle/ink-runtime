import {test} from 'node:test';
import assert from 'node:assert/strict';
import {readFileSync,writeFileSync,mkdtempSync,mkdirSync,rmSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {join} from 'node:path';
import {pathToFileURL,fileURLToPath} from 'node:url';
test('mixed module dispatches current graph and scalar aggregate host APIs',async()=>{
 const root=mkdtempSync(join(tmpdir(),'ink-module-dispatch-')),url=pathToFileURL(root+'/'),originalFetch=globalThis.fetch;
 try{
  for(const name of ['ink-module.mjs','initial-state.mjs'])writeFileSync(join(root,name),readFileSync(new URL('../hosts/'+name,import.meta.url)));
  writeFileSync(join(root,'state-wasm.mjs'),'export class StatefulModule{static async instantiate(){return{createSourceState(){return{pure(){return 17},dispose(){}}}}}}');
  writeFileSync(join(root,'program.mjs'),'export function call(){return 19}');
  writeFileSync(join(root,'cpu.wasm'),'stub');
  globalThis.__moduleDispatchClosed=[];
  const functions=[['sum',{aggregate:'sum',list_param:0,stages:[]}],['count',{aggregate:'count',list_param:0,stages:[]}],['ordered',{nodes:[{shader:'fold.wgsl'}]}]].map(([name,descriptor],index)=>({name,bundle:`part${index}/`,gpu:true,descriptor}));
  writeFileSync(join(root,'manifest.json'),JSON.stringify({cpu:'cpu.wasm',functions}));
  for(const[index,f]of functions.entries()){
   const part=join(root,`part${index}`);mkdirSync(part);
   writeFileSync(join(part,'ink-gpu.mjs'),index<2?`export async function loadInk(){return{run(name,args,backend){return Promise.resolve({value:args[0].length+${index},backend,reason:'scalar run'})},close(){globalThis.__moduleDispatchClosed.push(${index})}}}`:`export async function loadInk(){return{call(name,args,options){return Promise.resolve({value:args[0].length+${index},backend:options.backend,reason:'graph call'})},dispose(){globalThis.__moduleDispatchClosed.push(${index})}}}`);
  }
  globalThis.fetch=async u=>{const b=readFileSync(fileURLToPath(u));return{json:async()=>JSON.parse(b),arrayBuffer:async()=>b.buffer.slice(b.byteOffset,b.byteOffset+b.byteLength)}};
  const{loadInk}=await import(new URL('ink-module.mjs',url));const host=await loadInk(url,{gpu:true,policy:'static'});
  for(const[i,f]of functions.entries()){const r=await host.call(f.name,[[4,7]],{backend:'gpu'});assert.equal(r.backend,'gpu');assert.equal(r.value,2+i);assert.equal(r.reason,i<2?'scalar run':'graph call');}
  await host.dispose();assert.deepEqual(globalThis.__moduleDispatchClosed.sort(),[0,1,2]);
 }finally{delete globalThis.__moduleDispatchClosed;globalThis.fetch=originalFetch;rmSync(root,{recursive:true,force:true});}
});

import {StatefulModule} from './state-wasm.mjs';
import {readArtifact} from './initial-state.mjs';
// Complete source module. State execution is never benchmarked speculatively.
export async function loadInk(base,{gpu=true,backend='auto',...options}={}){
 const root=new URL(base,globalThis.location?.href??'http://localhost/');if(!root.pathname.endsWith('/'))root.pathname+='/';
 const manifest=await(await fetch(new URL('manifest.json',root))).json();
 const js=await import(new URL('program.mjs',root));
 const initialSnapshot=manifest.initial_state?await readArtifact(new URL(manifest.initial_state.file,root),manifest.initial_state):undefined;
 const cpu=await StatefulModule.instantiate(new Uint8Array(await(await fetch(new URL(manifest.cpu,root))).arrayBuffer()),{initialSnapshot});
 // Checked pure functions receive all data explicitly. Their invocation holder
 // needs no copy of the application's deployed model, optimizer or event outbox.
 const pure=cpu.createSourceState(),parts=new Map();
 async function part(f){if(!parts.has(f.name))parts.set(f.name,(async()=>{const url=new URL(f.bundle,root);const adapter=await import(new URL('ink-gpu.mjs',url));return adapter.loadInk(url,{gpu,backend,...options});})());return parts.get(f.name);}
 return {manifest,parallelStatistics:()=>({cpu:cpu.parallelStatistics(),javascript:js.parallelStatistics?.()}),createState:()=>cpu.create(),createJavaScriptState:()=>{if(!js.createState)throw Error('no state declarations');return js.createState();},restoreState:bytes=>cpu.restore(bytes),
  async call(name,args,{backend:chosen=backend,...callOptions}={}){
   if(!['auto','gpu','gpu-required','cpu','javascript'].includes(chosen))throw Error('invalid backend');
   const parallel=js.metadata?.functions?.find(f=>f.name===name)?.parallel??false;
   const cpuCall=()=>parallel?pure.pureAsync(name,args):pure.pure(name,args);
   const f=manifest.functions.find(f=>f.name===name);if(!f)throw Error(`unknown Ink function ${name}`);
   if(chosen==='javascript')return {value:parallel?await js.callAsync(name,args):js.call(name,args),backend:'javascript',reason:'explicit backend selection'};
   if(f.gpu&&chosen!=='cpu'){try{const host=await part(f);return await(host.call?host.call(name,args,{backend:chosen,...callOptions}):host.run(name,args,chosen));}catch(e){if(chosen==='gpu-required')throw e;return {value:await cpuCall(),backend:'cpu',reason:`GPU fallback: ${e.message}`};}}
   if(chosen==='gpu-required')throw Error(f.fallback_reason??'GPU lowering unavailable');
   return {value:await cpuCall(),backend:'cpu',reason:f.fallback_reason??'explicit backend selection'};
  },dispose(){pure.dispose();const pending=[...parts.values()].map(p=>p.then(host=>host.dispose?host.dispose():host.close?.()).catch(()=>{}));parts.clear();return Promise.all(pending);}
 };
}

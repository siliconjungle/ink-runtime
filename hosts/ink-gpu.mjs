import {createDevice} from "./ink-device.mjs";
import {createSelector} from "./backend-selection.mjs";
// Compiled Wasm CPU fallback + literal WGSL stages. Profiles never grant facts
// about future inputs. Calls are serialized; trials have no application effects.
const now=()=>performance.now();
function word(v,type){
  if(type==='bool'){if(typeof v!=='boolean')throw Error('expected Bool');return v;}
  if(type==='u32'){if(!Number.isInteger(v)||v<0||v>0xffffffff)throw Error('expected u32');return v;}
  if(typeof v==='number'&&!Number.isSafeInteger(v))throw Error('u64 Number must be an exact safe integer; use BigInt or a decimal string');
  if(typeof v!=='number'&&typeof v!=='bigint'&&!(typeof v==='string'&&/^\d+$/.test(v)))throw Error('expected u64');
  const n=BigInt(v);if(n<0n||n>0xffffffffffffffffn)throw Error('u64 outside range');return n;
}
function argumentsFor(f,args){
  if(!Array.isArray(args)||args.length!==f.params.length)throw Error('argument count mismatch');
  return args.map((v,i)=>{
    const t=f.params[i].type;
    if(!t.startsWith('list_'))return word(v,t);
    if(!Array.isArray(v)&&!(v instanceof Uint32Array)&&!(v instanceof BigUint64Array))throw Error('expected word list');
    const inner=t.slice(5);
    if(inner==='u32'&&v instanceof Uint32Array)return v.slice();
    if(inner==='u64'&&v instanceof BigUint64Array)return v.slice();
    const values=Array.from(v,x=>word(x,inner));
    return inner==='u32'?new Uint32Array(values):new BigUint64Array(values);
  });
}
function profileKey(f,args){
  const xs=args[f.gpu.list_param],n=xs.length;
  let zero=0,high=0,samples=0;
  for(let i=0;i<n;i+=Math.max(1,Math.floor(n/64))){zero+=xs[i]===0?1:0;high+=xs[i]>=0x80000000?1:0;samples++;if(samples===64)break;}
  return JSON.stringify([f.index,n?Math.floor(Math.log2(n)):0,Math.floor(zero*4/Math.max(1,samples)),Math.floor(high*4/Math.max(1,samples)),args.filter((_,i)=>i!==f.gpu.list_param)],(_,v)=>typeof v==='bigint'?v.toString():v);
}
export async function loadInk(base,options={}){
  const backend=options.backend??'auto';if(!['auto','cpu','javascript','gpu'].includes(backend))throw Error('backend must be auto, cpu, javascript or gpu');
  const baseURL=new URL(base,globalThis.location?.href??'http://localhost/');
  async function asset(name){const response=await fetch(new URL(name,baseURL));if(!response.ok)throw Error(`cannot load ${name}: ${response.status}`);return response;}
  const manifest=await (await asset('manifest.json')).json();
  if(manifest.schema!==1||manifest.backend!=='webgpu-wgpu-v1')throw Error('unsupported GPU bundle');
  const cpuStart=now();
  const {instance}=await WebAssembly.instantiate(await (await asset('cpu.wasm')).arrayBuffer(),{});
  const cpuStartup=now()-cpuStart;
  const functions=new Map(manifest.functions.map(f=>[f.name,f]));
  let closed=false,serial=Promise.resolve();
  const disabled=new Map();
  const selector=createSelector(options);
  const jsStart=now();const javascript=await import(new URL("program.mjs",baseURL));const jsStartup=now()-jsStart;
  const adapter=createDevice({asset,onLost:()=>selector.clear()});
  function cpu(f,args){
    const memory=instance.exports.memory;let top=Number(instance.exports.__heap_base.value);const abi=[];
    for(let i=0;i<args.length;i++){
      const v=args[i],t=f.params[i].type;
      if(t.startsWith('list_')){
        top=Math.ceil(top/8)*8;const end=top+v.byteLength;
        if(end>0xffffffff)throw Error('Wasm input exceeds address space');
        if(end>memory.buffer.byteLength)memory.grow(Math.ceil((end-memory.buffer.byteLength)/65536));
        new Uint8Array(memory.buffer,top,v.byteLength).set(new Uint8Array(v.buffer,v.byteOffset,v.byteLength));
        abi.push(top,v.length);top=end;
      }else abi.push(t==='bool'?(v?1:0):v);
    }
    const value=instance.exports[`lang_fn_${f.name}`](...abi);
    return f.result==='u32'?value>>>0:f.result==='u64'?BigInt.asUintN(64,value):Boolean(value);
  }
  async function run(f,args,backend){
    if(closed)throw Error('Ink runtime is closed');
    const candidates=[{name:'cpu',startupMs:cpuStartup,run:()=>cpu(f,args)}, {name:'javascript',startupMs:jsStartup,verify:(value,reference)=>value===reference,run:()=>javascript.call(f.name,args)}];
    if(f.gpu && !disabled.has(f.index) && options.gpu!==false) candidates.push({name:'gpu',async:true,verify:(value,reference)=>value===reference,prepare:()=>adapter.prepare(f,args),run:async()=>{
      try {return await adapter.run(f,args);}catch(error){if(!/limits|budget/.test(String(error)))disabled.set(f.index,String(error));throw error;}
    }});
    const key=f.gpu?profileKey(f,args):JSON.stringify([f.index,args.map(v=>typeof v==='object'?v.length:String(v))]);
    const outcome=await selector.run(key,candidates,backend);
    return {value:outcome.result,backend:outcome.backend,reason:outcome.reason,profile:outcome.profile};
  }
  return {
    manifest,
    capture(name,args){const f=functions.get(name);if(!f)throw Error(`unknown function ${name}`);return argumentsFor(f,args);},
    run(name,args,selection=backend){const f=functions.get(name);if(!f)return Promise.reject(Error(`unknown function ${name}`));let validated;try{if(!['auto','cpu','javascript','gpu'].includes(selection))throw Error('backend must be auto, cpu, javascript or gpu');validated=argumentsFor(f,args);}catch(e){return Promise.reject(e);}
      const pending=serial.then(()=>run(f,validated,selection));serial=pending.catch(()=>{});return pending;},
    async close(){await serial;closed=true;adapter.close();selector.clear();},
  };
}

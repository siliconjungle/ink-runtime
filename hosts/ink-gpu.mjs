import {createDevice} from "./ink-device.mjs";
// Compiled Wasm CPU fallback + literal WGSL stages. Profiles never grant facts
// about future inputs. Calls are serialized; trials have no application effects.
const median=xs=>[...xs].sort((a,b)=>a-b)[Math.floor(xs.length/2)];
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
  return JSON.stringify([f.index,n?Math.floor(Math.log2(n)):0,Math.floor(zero*4/Math.max(1,samples)),Math.floor(high*4/Math.max(1,samples)),args.filter((_,i)=>i!==f.gpu.list_param)]);
}
export async function loadInk(base,options={}){
  const backend=options.backend??'auto';if(!['auto','cpu','gpu'].includes(backend))throw Error('backend must be auto, cpu or gpu');
  const baseURL=new URL(base,globalThis.location?.href??'http://localhost/');
  async function asset(name){const response=await fetch(new URL(name,baseURL));if(!response.ok)throw Error(`cannot load ${name}: ${response.status}`);return response;}
  const manifest=await (await asset('manifest.json')).json();
  if(manifest.schema!==1||manifest.backend!=='webgpu-wgpu-v1')throw Error('unsupported GPU bundle');
  const {instance}=await WebAssembly.instantiate(await (await asset('cpu.wasm')).arrayBuffer(),{});
  const functions=new Map(manifest.functions.map(f=>[f.name,f]));
  let closed=false,serial=Promise.resolve();
  const profiles=new Map(),disabled=new Map();
  const adapter=createDevice({asset,onLost:()=>profiles.clear()});
  const horizon=options.expectedCalls??32,recheck=options.recheckEvery??32;
  if(!Number.isInteger(horizon)||horizon<1||horizon>100000||!Number.isInteger(recheck)||recheck<1||recheck>100000)throw Error('invalid profiling budget');
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
    if(disabled.has(f.index))return {value:cpu(f,args),backend:'cpu',reason:disabled.get(f.index)};
    if(backend==='cpu'||!f.gpu)return {value:cpu(f,args),backend:'cpu',reason:backend==='cpu'?'explicit CPU selection':f.cpu_reason};
    const key=profileKey(f,args);let profile=profiles.get(key);
    try{
      if(backend==='gpu')return {value:await adapter.run(f,args),backend:'gpu',reason:'explicit GPU selection'};
      if(!profile||profile.uses>=recheck){
        const setupStart=now();await adapter.prepare(f,args);const setup=now()-setupStart;
        const cpuTimes=[],gpuTimes=[];let reference;
        for(let trial=0;trial<3;trial++){
          const sampleCPU=()=>{
            // Batch very short CPU calls to avoid a zero timing from browser
            // clock quantisation. Bound calibration work independently of inputs.
            const start=now();let calls=0,elapsed;
            do{reference=cpu(f,args);calls++;elapsed=now()-start;}while(elapsed<0.5&&calls<256);
            cpuTimes.push(elapsed/calls);
          };
          const sampleGPU=async()=>{const start=now();const value=await adapter.run(f,args);gpuTimes.push(now()-start);return value;};
          let value;if(trial%2){value=await sampleGPU();sampleCPU();}else{sampleCPU();value=await sampleGPU();}
          if(value!==reference)throw Error('GPU/compiled CPU result mismatch');
        }
        const cpuMs=median(cpuTimes),gpuMs=median(gpuTimes);
        profile={cpu_ms:cpuMs,gpu_ms:gpuMs,gpu_setup_ms:setup,expected_calls:horizon,uses:0,backend:gpuMs+setup/horizon<cpuMs*0.9?'gpu':'cpu'};
        if(profiles.size>=32&&!profiles.has(key))profiles.delete(profiles.keys().next().value);
        profiles.set(key,profile);
      }
      profile.uses++;
      return {value:profile.backend==='gpu'?await adapter.run(f,args):cpu(f,args),backend:profile.backend,reason:'measured full execution cost with setup amortisation and 10% minimum saving',profile:{...profile}};
    }catch(error){
      // Invalid values were rejected before this point. Fallback repeats only
      // pure computations; no user events or authoritative state were changed.
      profiles.delete(key);
      if(!/limits|budget/.test(String(error)))disabled.set(f.index,`GPU fallback: ${String(error)}`);
      return {value:cpu(f,args),backend:'cpu',reason:`GPU fallback: ${String(error)}`};
    }
  }
  return {
    manifest,
    capture(name,args){const f=functions.get(name);if(!f)throw Error(`unknown function ${name}`);return argumentsFor(f,args);},
    run(name,args,selection=backend){const f=functions.get(name);if(!f)return Promise.reject(Error(`unknown function ${name}`));let validated;try{if(!['auto','cpu','gpu'].includes(selection))throw Error('backend must be auto, cpu or gpu');validated=argumentsFor(f,args);}catch(e){return Promise.reject(e);}
      const pending=serial.then(()=>run(f,validated,selection));serial=pending.catch(()=>{});return pending;},
    async close(){await serial;closed=true;adapter.close();profiles.clear();},
  };
}

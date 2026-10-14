// ROUTE is compiler-emitted from an immutable checked witness, not a runtime
// optimisation proposal. Loading code/assets faithfully remains host trust.
import {loadInk} from './ink-gpu.mjs';
export async function loadRoute(base,options={}){
  const loading=performance.now();
  const engine=await loadInk(base,{...options,backend:'cpu'});
  try{
    const manifest=engine.manifest,functions=new Map(manifest.functions.map(f=>[f.name,f]));
    const route=ROUTE,entry=functions.get(route.entry),types=new Map();
    if(route.schema!==1||route.semantics!=='ink-literal-source-routing-v1'||route.input_core_sha256!==manifest.core_sha256||!entry||route.stages.length>32)throw Error('incompatible compiled source route');
    function refType(r){
      if(Object.hasOwn(r,'Input'))return entry.params[r.Input]?.type;
      if(Object.hasOwn(r,'Stage'))return types.get(r.Stage);
      if(r.Literal){const keys=Object.keys(r.Literal);if(keys.length===1)return {U32:'u32',U64:'u64',Bool:'bool'}[keys[0]];}
      throw Error('invalid compiled route reference');
    }
    for(const s of route.stages){
      const f=functions.get(s.function);
      if(types.has(s.id)||!f||!((s.backend==='rust'&&s.domain==='cpu')||(s.backend==='gpu'&&s.domain==='gpu'))||s.arguments.length!==f.params.length||s.arguments.some((r,i)=>refType(r)!==f.params[i].type))throw Error('invalid compiled stage or typed edge');
      types.set(s.id,f.result);
    }
    if(refType(route.output)!==entry.result)throw Error('invalid compiled route result');
    const resourcePlan=typeof RESOURCES==='undefined'?null:RESOURCES;
    const releases=new Map((resourcePlan?.releases??[]).map(r=>[r.after,r]));
    const credits=resourcePlan?.admission;
    let serial=Promise.resolve(),closed=false,pendingCount=0,pendingBytes=0;
    function overload(){const error=Error('Ink compiled admission credits exhausted');error.code='INK_BACKPRESSURE';return error;}
    function knownPayloadBytes(args){
      if(!Array.isArray(args)||args.length!==entry.params.length)return null;
      let bytes=0;
      for(let i=0;i<args.length;i++){
        const type=entry.params[i].type,value=args[i];
        if(type.startsWith('list_')){
          if(!Array.isArray(value)&&!ArrayBuffer.isView(value))return null;
          const size=value.length*(type==='list_u64'?8:4);
          if(!Number.isSafeInteger(size))return null;bytes+=size;
        }else bytes+=type==='u64'?8:4;
      }
      return Number.isSafeInteger(bytes)?bytes:null;
    }
    function payloadBytes(args){return args.reduce((n,v,i)=>n+(entry.params[i].type.startsWith('list_')?v.byteLength:entry.params[i].type==='u64'?8:4),0);}
    function observe(event){try{if(typeof options.onUsage==='function')options.onUsage(Object.freeze(event));}catch{} /* instrumentation cannot change source behaviour */}

    if(typeof options.onUsage==='function')observe({source:manifest.usage_core_sha256??manifest.core_sha256,phase:'construct',event:'completed',complete_call_ns:String(Math.round((performance.now()-loading)*1e6))});
    function literal(r){if(Object.hasOwn(r,'U64'))return BigInt(r.U64);if(Object.hasOwn(r,'U32'))return r.U32;if(Object.hasOwn(r,'Bool'))return r.Bool;throw Error('invalid literal');}
    return {
      entry:route.entry,
      run(args){
        // Capture and validate the original signature synchronously, before any
        // asynchronous queue/device work; caller mutations cannot affect inputs.
        let owned,start=performance.now();try{if(closed)throw Error('Ink route runtime is closed');if(credits&&pendingCount>=credits.max_pending)throw overload();if(credits){const bytes=knownPayloadBytes(args);if(bytes!==null&&bytes>credits.max_pending_bytes-pendingBytes)throw overload();}owned=engine.capture(route.entry,args);}catch(e){return Promise.reject(e);}
        const recording=typeof options.onUsage==='function';
        const bytes=credits||recording?payloadBytes(owned):0;
        if(!Number.isSafeInteger(bytes)||credits&&bytes>credits.max_pending_bytes-pendingBytes)return Promise.reject(overload());
        pendingCount++;pendingBytes+=bytes;
        const captureMs=performance.now()-start,submitted=performance.now();
        const usage=recording?{source:manifest.usage_core_sha256??manifest.core_sha256,phase:'call:'+route.entry,function:route.entry,input_bytes:bytes,input_elements:owned.reduce((n,v)=>n+(typeof v==='object'?v.length:1),0),pending:pendingCount,pending_bytes:pendingBytes}:null;
        if(usage)observe({...usage,event:'accepted'});
        const pending=serial.then(async()=>{
          const start=performance.now(),values=new Map(),stages=[];
          const resolve=r=>Object.hasOwn(r,'Input')?owned[r.Input]:Object.hasOwn(r,'Stage')?values.get(r.Stage):literal(r.Literal);
          for(const s of route.stages){
            const bridgeStart=performance.now(),arguments_=s.arguments.map(resolve),bridgeMs=performance.now()-bridgeStart;
            const runStart=performance.now(),result=await engine.run(s.function,arguments_,s.domain==='cpu'?'cpu':'gpu');
            values.set(s.id,result.value);const release=releases.get(s.id);for(const id of release?.values??[])values.delete(id);for(const input of release?.inputs??[])owned[input]=undefined;stages.push({id:s.id,function:s.function,requested:s.domain,backend:result.backend,reason:result.reason,bridge_ms:bridgeMs,execution_ms:performance.now()-runStart});
          }
          return {value:resolve(route.output),stages,capture_ms:captureMs,queue_ms:start-submitted,execution_ms:performance.now()-start,total_ms:performance.now()-submitted+captureMs,scope:'pure values; scalar stage results materialise on host; physical ABI and backends trusted'};
        });
        const settled=pending.then(value=>{if(usage)observe({...usage,event:'completed',complete_call_ns:String(Math.round((performance.now()-submitted+captureMs)*1e6))});return value;},error=>{if(usage)observe({...usage,event:'failed'});throw error;}).finally(()=>{pendingCount--;pendingBytes-=bytes;owned=null;});
        serial=settled.catch(()=>{});return settled;
      },
      async close(){closed=true;await serial;const start=performance.now();await engine.close();if(typeof options.onUsage==='function')observe({source:manifest.usage_core_sha256??manifest.core_sha256,phase:'dispose',event:'completed',complete_call_ns:String(Math.round((performance.now()-start)*1e6))});},
    };
  }catch(e){await engine.close();throw e;}
}

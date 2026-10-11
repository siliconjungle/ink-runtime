import test from 'node:test';
import assert from 'node:assert/strict';
import {collectPlacementObservations,collectPlacementStartup} from '../hosts/pipeline-profile.mjs';
const plan=xs=>({inputs:{xs},steps:[{id:'a',call:'inc',args:[{input:'xs'}]},{id:'b',call:'double',args:[{step:'a'}]}],outputs:['b']});
function engine({wrong=false,fallback=false}={}) {
 const calls=[];
 return {calls,close:async()=>{},describePipeline:async()=>[{scope:'exact',boundary:'captured-segment',known:true,gpu:true,start:1,end:2,outputs:['b'],features:{input_elements:2,input_bytes:8,output_capacity_elements:2,dispatches:1,iterations:1}}],pipeline:async(p,options)=>{
  calls.push({p,options});const values={};
  for(const step of p.steps){const a=step.args[0],xs=a.step?values[a.step]:a.input?p.inputs[a.input]:a;values[step.id]=Array.from(xs,x=>step.call==='inc'?(x+1)>>>0:Math.imul(x,2)>>>0);}
  const requested=options.placements?.[0].backend??options.backend;
  if(wrong&&requested==='gpu')values.b[0]=999;
  return {values:Object.fromEntries(p.outputs.map(id=>[id,values[id]])),backend:fallback&&requested==='gpu'?'cpu':requested};
 }};
}
test('offline observations capture training inputs, materialise typed boundary ports and exclude fallback timings',async()=>{
 const e=engine({fallback:true}),xs=Uint32Array.of(1,2),p=plan(xs);
 const pending=collectPlacementObservations(e,[p],{backends:['cpu','gpu'],samples:1,warmup:0});xs.fill(99);p.steps[1].call='inc';
 const r=await pending;assert.equal(r.observations.length,1);assert.ok(r.observations[0].backends.cpu);assert.equal(r.observations[0].backends.gpu,undefined);
 assert.deepEqual(e.calls[1].p.inputs['step:a'],[2,3]);assert.equal(e.calls[1].p.steps[0].call,'double');assert.equal(r.requests,3);
});
test('wrong results and invalid collection budgets fail rather than becoming profile data',async()=>{
 await assert.rejects(collectPlacementObservations(engine({wrong:true}),[plan([1,2])],{backends:['cpu','gpu'],samples:1,warmup:0}),/mismatch/);
 for(const options of [{samples:0},{samples:32},{backends:['cpu','cpu']},{backends:['state']},{intervals:Array(65).fill([0,1])}])await assert.rejects(collectPlacementObservations(engine(),[plan([1,2])],options));
 await assert.rejects(collectPlacementObservations(engine(),Array(17).fill(plan([]))),/budget/);
});
test('startup collection creates and closes a fresh engine per trial and excludes unavailable targets',async()=>{
 let created=0,closed=0;
 const r=await collectPlacementStartup(async()=>{created++;const e=engine({fallback:true});e.close=async()=>{closed++};return e},plan([1,2]),{backends:['cpu','gpu'],trials:2});
 assert.equal(created,4);assert.equal(closed,4);assert.equal(r.records.cpu.cold_ns.length,2);assert.equal(r.records.cpu.warm_ns.length,6);assert.ok(r.setup_ns.cpu>=0);assert.equal(r.setup_ns.gpu,undefined);
});

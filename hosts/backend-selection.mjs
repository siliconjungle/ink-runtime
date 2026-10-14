// Decisions execute the same ink-abi Rust implementation used by native hosts.
import {selectionRequest} from './ink-selection.mjs';
import {pipelineScope} from './pipeline-contract.mjs';
export async function workloadFeatures(workload){return selectionRequest({operation:'features',workload});}
export function profileScope(manifest,entry){return JSON.stringify([manifest.core_sha256,manifest.cpu_rust_sha256,manifest.javascript_sha256,entry]);}
export async function validateCostProfile(profile,targetKey){await selectionRequest({operation:'validate',profile,target_key:targetKey});return profile==null?null:structuredClone(profile);}
export function createSelector({policy='adaptive',staticBackend='cpu',costProfile=null,targetKey=''}={}){
 const options=structuredClone({policy,static_backend:staticBackend,cost_profile:costProfile,target_key:targetKey});
 // Options are private and immutable. Reuse only the exact decision inputs;
 // changed eligibility, evidence boundary or workload gets a fresh ABI check.
 // Keep caller-owned results separate from the bounded cache.
 const decisions=new Map();
 const choose=async(eligible,context={})=>{
  const key=JSON.stringify({eligible,context});
  if(decisions.has(key))return JSON.parse(decisions.get(key));
  // Capture before the Wasm loader can yield; caller mutation must not bind
  // a result for a different workload under the earlier key.
  const decision=await selectionRequest({options,...JSON.parse(key)});
  if(decisions.size>=128)decisions.delete(decisions.keys().next().value);
  // ABI responses are JSON values. Retaining their encoding provides fresh
  // nested ownership on a hit without cloning the entire response graph.
  decisions.set(key,JSON.stringify(decision));return decision;
 };
 return {
  choose,
  clear(){decisions.clear();},
  async run(key,candidates,selection='auto',context={}){
   const required=selection==='gpu-required';if(required)selection='gpu';
   const baseline=candidates.find(c=>c.name==='cpu');if(!baseline)throw Error('compiled CPU baseline required');
   if(!['auto','cpu','javascript','gpu'].includes(selection))throw Error('invalid backend selection');
   let decision;
   if(selection==='auto')decision=await choose(candidates.map(c=>c.name),context);
   else decision={backend:selection,reason:'explicit backend selection'};
   let chosen=candidates.find(c=>c.name===decision.backend);
   if(!chosen){if(required)throw Error('GPU required but unavailable for this computation');if(selection==='gpu')decision={backend:'cpu',reason:'GPU unavailable for this computation'};else throw Error('backend unavailable for this computation');chosen=baseline;}
   try{if(chosen.prepare)await chosen.prepare();return{result:await chosen.run(),backend:chosen.name,reason:decision.reason,profile:decision};}
   catch(error){if(required||chosen===baseline)throw error;if(baseline.prepare)await baseline.prepare();return{result:await baseline.run(),backend:'cpu',reason:`${chosen.name} fallback: ${error.message??error}`};}
  }
 };
}

// A forecast of open -> steps -> full output reads -> close. This is cost
// evidence only: it does not restrict subsequent legal session operations.
// Partial reads, writes or an unknown horizon have no comparable model here.
export function sessionWorkload(value){
 if(value==null)return null;
 if(typeof value!=='object'||Array.isArray(value)||Object.keys(value).some(k=>!['iterations','readbacks'].includes(k)))throw Error('session workload requires iterations and optional full readbacks');
 const iterations=value.iterations,readbacks=value.readbacks??1;
 if(!Number.isSafeInteger(iterations)||iterations<1||iterations>1_000_000||!Number.isSafeInteger(readbacks)||readbacks<0||readbacks>1_000_000)throw Error('session workload count limit');
 return {iterations,readbacks};
}
export async function sessionContext(manifest,p,workload,dispatches,preparation=null){
 if(workload==null)return {};
 // A session ignores plan.iterations until step(n). Bind its actual topology,
 // including names -> constant slots, output ids and simultaneous feedback.
 // Distinct scope prevents ordinary-call/prepared-input profiles being reused.
 // Cold device/CPU initialization and missing shaders have different setup
 // costs. Unknown preparation state cannot match measured runtime contexts.
 const scope=JSON.stringify(['ink-resident-session-v2',pipelineScope(manifest,{...p,iterations:1}),p.steps.map(s=>s.id),[...p.inputs].map(([name,r])=>[name,r.index]),workload.readbacks,preparation]);
 const features=await workloadFeatures({inputs:p.constants.map(c=>({list:c.type.kind==='list',count:c.count,bytes:c.words.byteLength})),outputs:p.outputs.map(id=>(p.steps[p.ids.get(id)].count??0)*workload.readbacks),stages:[{dispatches}],iterations:workload.iterations});
 return {scope,boundary:'captured-owned',features};
}

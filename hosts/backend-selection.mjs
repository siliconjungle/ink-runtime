// Decisions execute the same ink-abi Rust implementation used by native hosts.
import {selectionRequest} from './ink-selection.mjs';
export async function workloadFeatures(workload){return selectionRequest({operation:'features',workload});}
export function profileScope(manifest,entry){return JSON.stringify([manifest.core_sha256,manifest.cpu_rust_sha256,manifest.javascript_sha256,entry]);}
export async function validateCostProfile(profile,targetKey){await selectionRequest({operation:'validate',profile,target_key:targetKey});return profile==null?null:structuredClone(profile);}
export function createSelector({policy='adaptive',staticBackend='cpu',costProfile=null,targetKey=''}={}){
 const options=structuredClone({policy,static_backend:staticBackend,cost_profile:costProfile,target_key:targetKey});
 const choose=(eligible,context={})=>selectionRequest({options,context,eligible});
 return {
  choose,
  clear(){},
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

import {selectionRequest} from './ink-selection.mjs';
// Tokens own a compiled function, captured typed context and immutable goal
// identity. They convey ranking predictions, never proof/migration authority.
const objectives=new WeakMap();
const hash=s=>typeof s==='string'&&/^[a-f0-9]{64}$/.test(s);
// Materialized Ink binary64 values preserve their raw bits at the public
// boundary. Convert only this bounded scalar before the shared score validator.
function scoreNumber(value){
 if(typeof value==='number')return value;
 if(!value||Object.keys(value).join(',')!=='F64Bits')throw Error('objective score scalar');
 const text=value.F64Bits;
 if(typeof text!=='string'||text.length>20||!/^(0|[1-9][0-9]*)$/.test(text))throw Error('objective score bits');
 const bits=BigInt(text);
 if(bits>18446744073709551615n)throw Error('objective score bits');
 const view=new DataView(new ArrayBuffer(8));view.setBigUint64(0,bits,true);
 return view.getFloat64(0,true);
}
const digest=async bytes=>[...new Uint8Array(await crypto.subtle.digest('SHA-256',bytes))].map(x=>x.toString(16).padStart(2,'0')).join('');

/** Prepare an ordinary emitted Ink fn(candidates, context)->List<Rank>.
 * Import the compiled module from a trusted deployment, as for other compiled
 * adapters. Module metadata/hashes are provenance, not code authentication.
 */
export async function prepareInkObjective(module,{entry,context,revision=0,direction='minimize',minimumImprovement=0}={}){
 const declaration=module.metadata?.functions?.find(f=>f.name===entry&&!f.internal);
 const codecs=Object.hasOwn(module.compiledValues??{},entry)?module.compiledValues[entry]:null;
 if(typeof entry!=='string'||!/^[A-Za-z0-9_.]{1,256}$/.test(entry)
    ||!hash(module.metadata?.core_sha256)||!declaration||declaration.params.length!==2
    ||declaration.params[0].type.kind!=='list'||declaration.result.kind!=='list'
    ||!codecs||typeof module.prepareCompiled!=='function'||typeof module.callPreparedCompiled!=='function')
   throw Error('objective requires a compiled pure two-argument list function');
 if(!Number.isSafeInteger(revision)||revision<0||!['minimize','maximize'].includes(direction)
    ||!Number.isFinite(minimumImprovement)||minimumImprovement<0||minimumImprovement>1e100)
   throw Error('objective identity/policy budget');
 // Capture and encode synchronously, before the digest can yield.
 const source=module.metadata.core_sha256;
 const contextCodec=codecs.params[1],candidateCodec=codecs.params[0],resultCodec=codecs.result;
 const captured=contextCodec.capture(context),bytes=contextCodec.encode(captured);
 if(bytes.byteLength>4_000_000)throw Error('objective context byte budget');
 const prepared=module.prepareCompiled(entry,[[1,captured]]);
 const execute=module.callPreparedCompiled;
 const configuration=Object.freeze({goal:Object.freeze({source,entry,revision,context:await digest(bytes)}),
   direction,minimum_improvement:minimumImprovement});
 const token=Object.freeze(Object.create(null));
 objectives.set(token,{configuration,evaluate(candidates){
   const input=candidateCodec.capture(candidates);
   const output=resultCodec.materialize(execute(prepared,[input]));
   if(!Array.isArray(output)||output.length>16)throw Error('objective result budget');
   const scores=Object.create(null);
   for(const row of output){
     if(!row||Object.keys(row).sort().join(',')!=='high,low,variant'
        ||typeof row.variant!=='string'||Object.hasOwn(scores,row.variant))
       throw Error('objective rank shape or duplicate variant');
     scores[row.variant]={low:scoreNumber(row.low),high:scoreNumber(row.high)};
   }
   return scores;
 }});
 return token;
}
export function captureObjective(token){
 if(token===null)return null;
 if(!objectives.has(token))throw Error('foreign or forged objective token');
 return token;
}
export function objectiveIdentity(token){
 const objective=objectives.get(captureObjective(token));
 return objective?structuredClone(objective.configuration.goal):null;
}
export async function objectiveDecision(token,adaptive,previousGoal=null){
 const objective=objectives.get(captureObjective(token));
 if(!objective)throw Error('objective token required');
 // Capture all inputs before either Wasm loading or source evaluation can yield.
 const request=structuredClone({schema:1,adaptive,...objective.configuration,previous_goal:previousGoal});
 const prepared=await selectionRequest({operation:'objective-prepare',request});
 const scores=objective.evaluate(prepared.candidates);
 return selectionRequest({operation:'objective-decide',request,
   assessment:{subject:prepared.subject,scores}});
}

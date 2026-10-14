import test from 'node:test';
import assert from 'node:assert/strict';
if(process.env.INK_ADAPTIVE_ABI){
 const {copyFileSync}=await import('node:fs');const {join}=await import('node:path');
 copyFileSync(join(process.env.INK_ADAPTIVE_ABI,'src/selection.mjs'),new URL('../hosts/ink-selection.mjs',import.meta.url));
 copyFileSync(join(process.env.INK_ADAPTIVE_ABI,'selection.wasm'),new URL('../hosts/ink-selection.wasm',import.meta.url));
}else await import('./shared-host-assets.mjs');
import {createAdaptiveRecorder} from '../hosts/adaptive-profile.mjs';
const {createAdaptiveState}=await import('../hosts/adaptive-state.mjs');
const enc=new TextEncoder(),dec=new TextDecoder();
function fixture(overrides={}){
 let executions=0;const created=[];
 function make(kind,bytes){const data=bytes?JSON.parse(dec.decode(bytes)):{value:0,version:0,events:[],ack:0};const instance={
  bind(name){return async(value)=>{executions++;if(name==='fail')throw Error('expected failure');if(name==='write'){data.value=value;data.version++;data.events.push(data.version);}return {value:data.value,version:data.version,events:[...data.events]};};},
  checkpoint(){return enc.encode(JSON.stringify(data));},acknowledge(c){data.ack=c;},dispose(){instance.disposed=true;},kind};created.push(instance);return instance;}
 const binding={source:'a'.repeat(64),compiler:'b'.repeat(64),target:'test',boundary:'owned-serial-state-v1',variants:{direct:'c'.repeat(64),indexed:'d'.repeat(64)}};
 const cells=(r,w)=>({'action:read@s':{low_ns:r,high_ns:r},'action:write@s':{low_ns:w,high_ns:w}});
 const profile={binding,variants:{direct:{cells:cells(1000,1),peak_bytes:100},indexed:{cells:cells(1,1000),peak_bytes:200}},migrations:['direct','indexed'].map(from=>({from,to:from==='direct'?'indexed':'direct',high_ns:1,peak_bytes:300,max_pause_ns:1,min_state_elements:0,max_state_elements:100}))};
 const variants={direct:{create:()=>make('direct'),restore:b=>make('direct',b)},indexed:{create:()=>make('indexed'),restore:b=>make('indexed',b)}};
 const policy={horizon_calls:100,min_residence_calls:8,stable_windows:2,min_gain:.15,migration_margin:2,memory_limit_bytes:500,max_pause_ns:1e9,max_switches:4};
 return {binding,profile,variants,baseline:'direct',policy,recording:{windowCalls:4,sampleEvery:2},executions:()=>executions,created,...overrides};
}
const features={shape:'s',state_elements:1};
test('recent windows, failures, saturation, clock faults, timing budgets and detached history',()=>{
 let time=0;const r=createAdaptiveRecorder({windowCalls:2,maxCells:1,historyWindows:1,sampleEvery:1,maxObservationNs:1,clock:()=>++time});
 let s=r.start();assert.equal(r.finish('read','s',s),null);s=r.start();assert.equal(r.finish('write','s',s).saturated,true);assert.equal(r.snapshot().timing_enabled,false);
 r.finish('read','s',null);r.finish('read','s',null);assert.equal(r.snapshot().windows.length,1);assert.equal(r.snapshot().windows[0].saturated,false);
 const detached=r.snapshot();detached.windows[0].counts={};assert.equal(r.snapshot().windows[0].counts['read@s'],2);
 const broken=createAdaptiveRecorder({windowCalls:1,sampleEvery:1,clock(){throw Error('clock');}});assert.equal(broken.finish('read','s',broken.start(),{failed:true}).counts['failed:read@s'],1);
});
test('phase shift switches both ways with exact snapshots, events and no repeated calls',async()=>{
 const f=fixture(),events=[];const host=await createAdaptiveState({...f,onDiagnostic:e=>events.push(e)});
 await host.call('write',[7],features);await host.acknowledge(1,0);
 const before=await host.checkpoint();for(let i=0;i<12;i++)assert.equal((await host.call('read',[],features)).value,7);
 assert.equal(host.status().current,'indexed');assert.deepEqual(await host.checkpoint(),before);
 for(let i=0;i<12;i++)await host.call('write',[i],features);
 assert.equal(host.status().current,'direct');assert.equal(f.executions(),25);assert.equal(events.filter(e=>e.kind==='switched').length,2);
 assert.equal(f.created.filter(x=>x.disposed).length,2);await host.close();assert.ok(f.created.every(x=>x.disposed));
});
test('failed/corrupt/over-budget migration retains old instance and original reply',async()=>{
 for(const mode of ['throw','corrupt','snapshot','pause']){
  const f=fixture();if(mode==='throw')f.variants.indexed.restore=()=>{throw Error('restore failed');};
  if(mode==='corrupt')f.variants.indexed.restore=()=>f.variants.indexed.create();
  let t=0;const host=await createAdaptiveState({...f,...(mode==='snapshot'?{maxSnapshotBytes:1}:{}),...(mode==='pause'?{clock:()=>t+=1e6}:{}),onDiagnostic(){throw Error('observer');}});
  await host.call('write',[123],features);for(let i=0;i<12;i++)assert.equal((await host.call('read',[],features)).value,123);
  assert.equal(host.status().current,'direct',mode);assert.equal(host.status().disabled,true,mode);assert.equal(f.executions(),13);await host.close();
 }
});
test('unknown cells retain baseline; errors stay errors; static mode stays fixed',async()=>{
 const f=fixture(),host=await createAdaptiveState(f);for(let i=0;i<12;i++)await host.call('read');assert.equal(host.status().current,'direct');await assert.rejects(host.call('fail'),/expected failure/);await host.close();
 const fixed=await createAdaptiveState({...fixture(),enabled:false});for(let i=0;i<12;i++)await fixed.call('read',[],features);assert.equal(fixed.status().current,'direct');await fixed.close();
});
test('serialized captured inputs, bounded queue and close drain accepted calls',async()=>{
 const f=fixture(),host=await createAdaptiveState({...f,maxPending:2});const a=[5],p=host.call('write',a,features);a[0]=99;const q=host.call('read',[],features);assert.throws(()=>host.call('read'),/BACKPRESSURE/);
 const closing=host.close();assert.equal((await p).value,5);assert.equal((await q).value,5);await closing;assert.throws(()=>host.call('read'),/BACKPRESSURE/);
});
test('external restore discards workload assumptions; malformed binding rejects before construction',async()=>{
 const f=fixture(),host=await createAdaptiveState(f),snap=await host.checkpoint();await host.call('write',[8],features);await host.restore(snap);assert.equal((await host.call('read')).value,0);assert.ok(host.status().disabled);await host.close();
 const bad=fixture();bad.profile=structuredClone(bad.profile);bad.profile.binding.compiler='f'.repeat(64);await assert.rejects(createAdaptiveState(bad),/binding/);assert.equal(bad.created.length,0);
});

test('a completed call releases its single queue slot before its promise resolves',async()=>{
 const host=await createAdaptiveState({...fixture(),maxPending:1});
 for(let i=0;i<10;i++)await host.call('read',[],features);
 assert.equal(host.status().pending,0);await host.close();
});

test('no recording means no timings, feature windows or controller work',async()=>{
 const host=await createAdaptiveState({...fixture(),enabled:false,recording:false,clock(){throw Error('clock must not run');}});
 for(let i=0;i<20;i++)await host.call('read');assert.equal(host.status().usage,null);assert.equal(host.status().access,null);await host.close();
 await assert.rejects(createAdaptiveState({...fixture(),recording:false}),/requires recording/);
});
test('usage samples measure locality and changing access mix without inspecting arguments',async()=>{
 const host=await createAdaptiveState({...fixture(),enabled:false,recording:false,usage:{windowCalls:4,sampleEvery:1}});
 for(let i=0;i<4;i++)await host.call('read',[],{usage:{kind:'read',key:1,path:2,size:128,scanned:100}});
 const first=host.status().access.windows[0];assert.equal(first.summary.repeated_keys,3);assert.equal(first.summary.scanned,400);
 for(let i=0;i<4;i++)await host.call('write',[i],{usage:{kind:'write',key:i,invalidated:1}});
 assert.equal(host.status().access.windows[1].operation_mix_drift,1);await host.close();
});
test('independent collections switch locally and keep unrelated snapshots untouched',async()=>{
 const {createAdaptiveCollections}=await import('../hosts/adaptive-collections.mjs');
 const hot=fixture(),cold=fixture(),group=await createAdaptiveCollections({hot,cold});
 await group.call('cold','write',[99],features);const before=await group.checkpoint('cold');
 for(let i=0;i<12;i++)await group.call('hot','read',[],features);
 assert.equal(group.status().hot.current,'indexed');assert.equal(group.status().cold.current,'direct');
 assert.equal(cold.created.length,1);assert.deepEqual(await group.checkpoint('cold'),before);assert.throws(()=>group.call('absent','read'),/unknown collection/);await group.close();
});
await import('./usage-window.mjs');

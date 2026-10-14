import test from 'node:test';
import assert from 'node:assert/strict';
// Generated ABI assets must exist before Node resolves the host module graph.
await import('./shared-host-assets.mjs');
const {completeSession}=await import('../hosts/complete-session.mjs');
const {prepareCostProfile}=await import('../hosts/backend-selection.mjs');

function fixture(failure){
 const events=[],context={scope:'engine',boundary:'captured-owned',features:{iterations:3}};
 let live=16,opened,settings;
 const act=name=>{events.push(name);if(failure===name)throw Error(name+' failed');};
 const session={backend:'gpu',reason:'saved engine profile',profile:{backend:'gpu'},costContext:context,
  counters:{upload_bytes:16,get live_buffer_bytes(){return live;},peak_buffer_bytes:24},
  async step(n){act('step');assert.equal(n,3);return {counters:{dispatches:n}};},
  async finish(){act('finish');},
  async read(request){act('read');assert.deepEqual(request,{outputs:['next']});return {values:{next:[7,8]},counters:{readback_bytes:8}};},
  async close(){live=0;act('session-close');}};
 return {events,context,get opened(){return opened;},get settings(){return settings;},async load(base,options){
  settings=options;act('load');await Promise.resolve();
  return {async open(plan,options){act('open');opened={plan,options};return session;},async close(){act('engine-close');}};
 }};
}
test('complete execution owns inputs, one session, one read and both resource lifetimes',async()=>{
 const f=fixture(),plan={inputs:{rows:new Uint32Array([1,2])},steps:[{id:'next'}],iterations:3};
 const costProfile={models:{}};
 const pending=completeSession(f.load,'bundle/',plan,{costProfile});
 plan.inputs.rows[0]=99;plan.steps[0].id='changed';costProfile.models.changed=true;
 const result=await pending;
 assert.equal(f.opened.plan.inputs.rows[0],1);assert.deepEqual(f.settings.costProfile,{models:{}});
 assert.deepEqual(f.opened.options,{backend:'auto',workload:{iterations:3,readbacks:1}});
 assert.deepEqual(f.events,['load','open','step','finish','read','session-close','engine-close']);
 assert.deepEqual(result.values,{next:[7,8]});assert.deepEqual(result.context,f.context);
 assert.deepEqual(result.counters,{upload_bytes:16,live_buffer_bytes:0,peak_buffer_bytes:24,dispatches:3,readback_bytes:8});
});
test('every acquired resource is closed on open, execution, read and disposal failures',async()=>{
 for(const failure of ['load','open','step','finish','read','session-close','engine-close']){
  const f=fixture(failure);
  await assert.rejects(completeSession(f.load,'bundle/',{steps:[{id:'next'}]},{iterations:3}),new RegExp(failure+' failed'));
  assert.equal(f.events.filter(n=>n==='engine-close').length,failure==='load'?0:1);
  assert.equal(f.events.filter(n=>n==='session-close').length,['load','open'].includes(failure)?0:1);
 }
});
test('invalid complete workloads reject before allocating an engine',async()=>{
 for(const options of [{iterations:0},{iterations:1.5},{iterations:true},{iterations:1_000_001},{iterations:3,backend:'javascript'}]){
  const f=fixture();await assert.rejects(completeSession(f.load,'bundle/',{},options),/complete session/);assert.deepEqual(f.events,[]);
 }
});
test('complete execution retains an already captured immutable profile token',async()=>{
 const prepared=await prepareCostProfile({schema:1,target_key:'device',models:{}},'device');
 const f=fixture();
 await completeSession(f.load,'bundle/',{steps:[{id:'next'}]},{iterations:3,costProfile:prepared});
 assert.equal(f.settings.costProfile,prepared);
});

import test from 'node:test';
import assert from 'node:assert/strict';
import './shared-host-assets.mjs';
const {createSelector}=await import('../hosts/backend-selection.mjs');
const costProfile={schema:1,target_key:'test-device',models:{kernel:{boundary:'captured-owned',domains:{input_elements:[0,65536],input_bytes:[0,262144]},backends:{cpu:{intercept_ns:100,terms:{input_elements:10}},javascript:{intercept_ns:20,terms:{input_elements:20}},gpu:{intercept_ns:1000,terms:{input_elements:1}}}}}};
test('adaptive saved profiles choose by size without probing or reading the timer',async()=>{
 const calls={cpu:0,javascript:0,gpu:0},prepared={cpu:0,javascript:0,gpu:0};
 const candidates=Object.keys(calls).map(name=>({name,prepare(){prepared[name]++},run(){calls[name]++;return 7}}));
 const selector=createSelector({costProfile,targetKey:'test-device',clock(){throw Error('online timer must not be read')}});
 const context=n=>({scope:'kernel',boundary:'captured-owned',features:{input_elements:n,input_bytes:n*4}});
 assert.equal((await selector.run('kernel',candidates,'auto',context(1))).backend,'javascript');
 assert.equal((await selector.run('kernel',candidates,'auto',context(1000))).backend,'gpu');
 assert.deepEqual(calls,{cpu:0,javascript:1,gpu:1});assert.deepEqual(prepared,calls);
 for(let i=0;i<40;i++)await selector.run('kernel',candidates,'auto',context(1000));
 assert.deepEqual(calls,{cpu:0,javascript:1,gpu:41});
 assert.equal((await selector.run('kernel',candidates,'auto',context(65537))).backend,'cpu');
 assert.equal((await selector.run('kernel',candidates,'auto',{...context(1),boundary:'resident-input-owned-readback'})).backend,'cpu');
});
test('static bypasses profiles and adaptive requires comparable CPU evidence',async()=>{
 const candidates=['cpu','javascript','gpu'].map(name=>({name,run:()=>7}));
 const selector=createSelector({policy:'static',staticBackend:'javascript',costProfile,targetKey:'test-device',clock(){throw Error('no timer')}});
 assert.equal((await selector.run('kernel',candidates)).backend,'javascript');
 assert.equal((await selector.run('kernel',candidates,'cpu')).backend,'cpu');
 assert.equal((await selector.run('kernel',candidates.slice(0,1))).backend,'cpu');
 const profile=structuredClone(costProfile);delete profile.models.kernel.backends.cpu;
 const adaptive=createSelector({costProfile:profile,targetKey:'test-device'});
 assert.equal((await adaptive.run('kernel',candidates,'auto',{scope:'kernel',boundary:'captured-owned',features:{input_elements:1,input_bytes:4}})).backend,'cpu');
 await assert.rejects(createSelector({costProfile,targetKey:'wrong-device'}).run('kernel',candidates),/target/);
});
test('profile is privately bound and device failure keeps compiled CPU without speculative verification',async()=>{
 const input=structuredClone(costProfile),calls={cpu:0,gpu:0};
 const selector=createSelector({costProfile:input,targetKey:'test-device'});input.models.kernel.backends.gpu.intercept_ns=1e12;
 const candidates=[{name:'cpu',prepare(){},run(){calls.cpu++;return 7}},{name:'gpu',prepare(){throw Error('device lost')},run(){calls.gpu++;return 8}}];
 const result=await selector.run('kernel',candidates,'auto',{scope:'kernel',boundary:'captured-owned',features:{input_elements:1000,input_bytes:4000}});
 assert.equal(result.backend,'cpu');assert.deepEqual(calls,{cpu:1,gpu:0});assert.match(result.reason,/fallback/);
});
test('adaptive selects the minimum eligible estimate with margin measured against CPU',async()=>{
 const input=structuredClone(costProfile);
 input.models.kernel.backends={cpu:{intercept_ns:100,terms:{}},javascript:{intercept_ns:80,terms:{}},gpu:{intercept_ns:75,terms:{}}};
 const context={scope:'kernel',boundary:'captured-owned',features:{input_elements:1,input_bytes:4}};
 for(const names of [['cpu','javascript','gpu'],['gpu','javascript','cpu']]){
  const selector=createSelector({costProfile:input,targetKey:'test-device'});
  assert.equal((await selector.run('kernel',names.map(name=>({name,run:()=>7})),'auto',context)).backend,'gpu');
 }
});

test('required GPU execution never invokes a CPU fallback',async()=>{
 const selector=createSelector();let calls=0;const cpu={name:'cpu',run:()=>{calls++;return 7;}};
 await assert.rejects(selector.run('missing',[cpu],'gpu-required'),/unavailable/);
 await assert.rejects(selector.run('lost',[cpu,{name:'gpu',run:()=>{throw Error('device lost');}}],'gpu-required'),/device lost/);
 assert.equal(calls,0);
 assert.equal((await selector.run('working',[cpu,{name:'gpu',run:()=>7}],'gpu-required')).backend,'gpu');
});

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

test('reused decisions preserve private ownership and recheck changed eligibility and boundaries',async()=>{
 const selector=createSelector({costProfile,targetKey:'test-device'});
 const context={scope:'kernel',boundary:'captured-owned',features:{input_elements:1000,input_bytes:4000}};
 const first=await selector.choose(['cpu','gpu'],context);assert.equal(first.backend,'gpu');
 first.backend='javascript';first.reason='caller mutation';
 assert.equal((await selector.choose(['cpu','gpu'],context)).backend,'gpu');
 assert.equal((await selector.choose(['cpu'],context)).backend,'cpu');
 assert.equal((await selector.choose(['cpu','gpu'],{...context,boundary:'resident-input-owned-readback'})).backend,'cpu');
 for(let i=0;i<130;i++)await selector.choose(['cpu','gpu'],{...context,scope:'other-'+i});
 assert.equal((await selector.choose(['cpu','gpu'],context)).backend,'gpu');
 selector.clear();assert.equal((await selector.choose(['cpu','gpu'],context)).backend,'gpu');
 await assert.rejects(selector.choose(['gpu'],context),/CPU baseline/);
 const changing={...context,features:{input_elements:1,input_bytes:4}},eligible=['cpu','gpu'];
 const pending=selector.choose(eligible,changing);
 changing.features.input_elements=1000;changing.features.input_bytes=4000;eligible.pop();
 assert.equal((await pending).backend,'cpu');
 assert.equal((await selector.choose(['cpu','gpu'],{...context,features:{input_elements:1,input_bytes:4}})).backend,'cpu');
});

// Adaptive state uses the same native/Wasm decision boundary.
import './adaptive-state.mjs';

const {sessionWorkload,sessionContext}=await import('../hosts/backend-selection.mjs');
const sessionManifest={core_sha256:'core',cpu_c_sha256:'cpu',javascript_sha256:'js',runtime_host_sha256:'host',shaders:['shader']};
function sessionPlan(){
 const type={kind:'list',element:{kind:'u32'}};
 return {constants:[{type,count:100,words:new Uint32Array(100)}],inputs:new Map([['rows',{index:0}]]),
  steps:[{id:'updated',f:{name:'update'},args:[{input:'rows',kind:'constant',index:0}],count:100}],
  ids:new Map([['updated',0]]),outputs:['updated'],feedback:new Map([['rows',0]]),iterations:999};
}
test('resident lifecycle costs charge setup once and steps and complete reads explicitly',async()=>{
 const p=sessionPlan(),short=await sessionContext(sessionManifest,p,sessionWorkload({iterations:1}),1),long=await sessionContext(sessionManifest,p,sessionWorkload({iterations:32}),1);
 assert.equal(short.scope,long.scope);assert.equal(short.features.input_bytes,400);
 assert.equal(long.features.input_bytes,400);assert.equal(long.features.dispatches,32);
 assert.equal(long.features.output_capacity_elements,100);assert.equal(long.features.iterations,32);
 const reads=await sessionContext(sessionManifest,p,sessionWorkload({iterations:32,readbacks:3}),1);
 assert.equal(reads.features.output_capacity_elements,300);assert.notEqual(reads.scope,long.scope);
 const profile={schema:1,target_key:'session-device',models:{[short.scope]:{boundary:short.boundary,domains:{iterations:[1,32]},backends:{cpu:{intercept_ns:10,terms:{iterations:100}},gpu:{intercept_ns:1000,terms:{iterations:1}}}}}};
 const selector=createSelector({costProfile:profile,targetKey:'session-device'});
 assert.equal((await selector.choose(['cpu','gpu'],short)).backend,'cpu');
 assert.equal((await selector.choose(['cpu','gpu'],long)).backend,'gpu');
 assert.equal((await selector.choose(['cpu','gpu'],reads)).backend,'cpu');
 assert.equal((await selector.choose(['cpu','gpu'],await sessionContext(sessionManifest,p,null,1))).backend,'cpu');
 const outside=await sessionContext(sessionManifest,p,sessionWorkload({iterations:33}),1);
 assert.equal((await selector.choose(['cpu','gpu'],outside)).backend,'cpu');
});
test('session evidence binds feedback, output identity, input binding and compiled host',async()=>{
 const context=async(p,m=sessionManifest)=>sessionContext(m,p,sessionWorkload({iterations:32}),1);
 const original=await context(sessionPlan());
 for(const mutate of [p=>p.feedback.clear(),p=>p.inputs.set('rows',{index:1}),p=>p.steps[0].id='other']){
  const p=sessionPlan();mutate(p);assert.notEqual((await context(p)).scope,original.scope);
 }
 assert.notEqual((await context(sessionPlan(),{...sessionManifest,runtime_host_sha256:'changed'})).scope,original.scope);
 const p=sessionPlan();p.iterations=1;assert.equal((await context(p)).scope,original.scope);
 const ordinary={schema:1,target_key:'session-device',models:{kernel:{boundary:'captured-owned',domains:{},backends:{cpu:{intercept_ns:100,terms:{}},gpu:{intercept_ns:1,terms:{}}}}}};
 assert.equal((await createSelector({costProfile:ordinary,targetKey:'session-device'}).choose(['cpu','gpu'],original)).backend,'cpu');
});
test('session workload forecasts are bounded, copied, and do not accept unsupported writes',()=>{
 for(const value of [{},{iterations:0},{iterations:true},{iterations:1.5},{iterations:1_000_001},{iterations:32,readbacks:-1},{iterations:32,readbacks:true},{iterations:32,writes:1},[]])assert.throws(()=>sessionWorkload(value),/session workload/);
 assert.deepEqual(sessionWorkload({iterations:32,readbacks:0}),{iterations:32,readbacks:0});
 const original={iterations:32},copied=sessionWorkload(original);original.iterations=1;
 assert.equal(copied.iterations,32);assert.equal(sessionWorkload(null),null);
});

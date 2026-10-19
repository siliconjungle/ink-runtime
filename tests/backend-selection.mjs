import test from 'node:test';
import assert from 'node:assert/strict';
import './shared-host-assets.mjs';
const {createSelector,profileScope}=await import('../hosts/backend-selection.mjs');
const {prepareCostProfile,captureCostProfile}=await import('../hosts/backend-selection.mjs');
const costProfile={schema:1,target_key:'test-device',models:{kernel:{boundary:'captured-owned',domains:{input_elements:[0,65536],input_bytes:[0,262144]},backends:{cpu:{intercept_ns:100,terms:{input_elements:10}},javascript:{intercept_ns:20,terms:{input_elements:20}},gpu:{intercept_ns:1000,terms:{input_elements:1}}}}}};
test('whole-call prices bind changed runtime transport and the scope version',()=>{
 const manifest={core_sha256:'core',cpu_rust_sha256:'cpu',javascript_sha256:'js',runtime_host_sha256:'host'};
 assert.deepEqual(JSON.parse(profileScope(manifest,'entry')),['ink-complete-call-v2','core','cpu','js','host','entry']);
 assert.notEqual(profileScope(manifest,'entry'),profileScope({...manifest,runtime_host_sha256:'changed'},'entry'));
});
test('prepared profiles capture before yielding and preserve shared decisions and target rejection',async()=>{
 const input=structuredClone(costProfile),pending=prepareCostProfile(input,'test-device');
 input.models.kernel.backends.gpu.intercept_ns=1e12;
 const prepared=await pending;
 assert.ok(Object.isFrozen(prepared));assert.deepEqual(Object.keys(prepared),[]);
 assert.equal(captureCostProfile(prepared),prepared);
 assert.equal(await prepareCostProfile(null,'test-device'),null);
 for(const n of [1,1000,65537])for(const eligible of [['cpu'],['cpu','gpu'],['cpu','javascript','gpu']]){
  const context={scope:'kernel',boundary:'captured-owned',features:{input_elements:n,input_bytes:4*n}};
  const expected=await createSelector({costProfile,targetKey:'test-device'}).choose(eligible,context);
  assert.deepEqual(await createSelector({costProfile:prepared,targetKey:'test-device'}).choose(eligible,context),expected);
 }
 await assert.rejects(createSelector({costProfile:prepared,targetKey:'wrong'}).choose(['cpu','gpu']),/target/);
 await assert.rejects(prepareCostProfile(prepared,'wrong'),/target/);
 await assert.rejects(createSelector({costProfile:structuredClone(prepared),targetKey:'test-device'}).choose(['cpu']),/target\/schema/);
 const rebound=await prepareCostProfile(prepared,'test-device');
 assert.notEqual(prepared,rebound);
});

test('prepared profile reuse avoids cloning its model graph and retains owned results',async()=>{
 const prepared=await prepareCostProfile(costProfile,'test-device');
 const original=globalThis.structuredClone;let modelCopies=0;
 globalThis.structuredClone=value=>{if(value?.models||value?.cost_profile?.models)modelCopies++;return original(value);};
 try{
  const context={scope:'kernel',boundary:'captured-owned',features:{input_elements:1000,input_bytes:4000}};
  const a=createSelector({costProfile:captureCostProfile(prepared),targetKey:'test-device'});
  const first=await a.choose(['cpu','gpu'],context);first.backend='changed';
  const b=createSelector({costProfile:captureCostProfile(prepared),targetKey:'test-device'});
  assert.equal((await b.choose(['cpu','gpu'],context)).backend,'gpu');
  a.clear();assert.equal((await b.choose(['cpu','gpu'],context)).backend,'gpu');
  assert.equal(modelCopies,0);
 }finally{globalThis.structuredClone=original;}
});
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
import './complete-session.mjs';

const {sessionWorkload,sessionContext}=await import('../hosts/backend-selection.mjs');
const sessionManifest={core_sha256:'core',cpu_rust_sha256:'cpu',javascript_sha256:'js',runtime_host_sha256:'host',shaders:['shader']};
function sessionPlan(){
 const type={kind:'list',element:{kind:'u32'}};
 return {constants:[{type,count:100,words:new Uint32Array(100)}],inputs:new Map([['rows',{index:0}]]),
  steps:[{id:'updated',f:{name:'update'},args:[{input:'rows',kind:'constant',index:0}],count:100}],
  ids:new Map([['updated',0]]),outputs:['updated'],feedback:new Map([['rows',0]]),iterations:999};
}
test('complete engine profiles cannot authorise resident session decisions or the reverse',async()=>{
 const p=sessionPlan(),forecast=sessionWorkload({iterations:32});
 const preparation={cpu:false,gpu:{device:false,shaders:[['update',false]]}};
 const session=await sessionContext(sessionManifest,p,forecast,1,preparation);
 const engine=await sessionContext(sessionManifest,p,forecast,1,preparation,'engine');
 assert.notEqual(engine.scope,session.scope);assert.deepEqual(engine.features,session.features);
 for(const [measured,other]of [[session,engine],[engine,session]]){
  const profile={schema:1,target_key:'device',models:{[measured.scope]:{boundary:measured.boundary,domains:{iterations:[1,32]},backends:{cpu:{intercept_ns:100,terms:{}},gpu:{intercept_ns:1,terms:{}}}}}};
  const selector=createSelector({targetKey:'device',costProfile:profile});
  assert.equal((await selector.choose(['cpu','gpu'],measured)).backend,'gpu');
  assert.equal((await selector.choose(['cpu','gpu'],other)).backend,'cpu');
 }
 await assert.rejects(sessionContext(sessionManifest,p,forecast,1,preparation,'kernel-only'),/objective/);
});
test('resident profiles never transfer warm costs to cold CPU, device or shaders',async()=>{
 const p=sessionPlan(),workload=sessionWorkload({iterations:32});
 const warm={cpu:true,gpu:{device:true,shaders:[['update',true]]}};
 const context=preparation=>sessionContext(sessionManifest,p,workload,1,preparation);
 const measured=await context(warm);
 const profile={schema:1,target_key:'warm-device',models:{[measured.scope]:{boundary:measured.boundary,domains:{iterations:[1,32]},backends:{cpu:{intercept_ns:100,terms:{}},gpu:{intercept_ns:1,terms:{}}}}}};
 const selector=createSelector({costProfile:profile,targetKey:'warm-device'});
 assert.equal((await selector.choose(['cpu','gpu'],measured)).backend,'gpu');
 for(const preparation of [null,{...warm,cpu:false},{cpu:true,gpu:{device:false,shaders:[['update',false]]}},{cpu:true,gpu:{device:true,shaders:[['update',false]]}}]){
  const changed=await context(preparation);assert.notEqual(changed.scope,measured.scope);
  assert.equal((await selector.choose(['cpu','gpu'],changed)).backend,'cpu');
 }
 assert.equal((await selector.choose(['cpu','gpu'],await context(warm))).backend,'gpu');
});
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

test('separate selectors reuse only complete immutable decision inputs',async()=>{
 const context={scope:'kernel',boundary:'captured-owned',features:{input_elements:1000,input_bytes:4000}};
 const first=createSelector({costProfile:structuredClone(costProfile),targetKey:'test-device'});
 const expected=await first.choose(['cpu','gpu'],context);assert.equal(expected.backend,'gpu');
 expected.cost_ns.cpu=-1;expected.reason='caller-owned mutation';
 const second=createSelector({costProfile:structuredClone(costProfile),targetKey:'test-device'});
 const repeated=await second.choose(['cpu','gpu'],context);
 assert.equal(repeated.backend,'gpu');assert.equal(repeated.cost_ns.cpu,10100);assert.notEqual(repeated.reason,expected.reason);
 const changed=structuredClone(costProfile);changed.models.kernel.backends.gpu.intercept_ns=1e12;
 assert.equal((await createSelector({costProfile:changed,targetKey:'test-device'}).choose(['cpu','gpu'],context)).backend,'cpu');
 assert.equal((await second.choose(['cpu'],context)).backend,'cpu');
 assert.equal((await second.choose(['cpu','gpu'],{...context,scope:'unmeasured'})).backend,'cpu');
 await assert.rejects(createSelector({costProfile,targetKey:'different-target'}).choose(['cpu','gpu'],context),/target/);
 await assert.rejects(createSelector({costProfile,targetKey:'test-device',policy:'probe'}).choose(['cpu','gpu'],context),/policy/);
 // Eviction and explicit clearing may add checks, never change the decision.
 for(let i=0;i<270;i++){
  const p=structuredClone(costProfile);p.models.kernel.backends.gpu.intercept_ns=i*100;
  const result=await createSelector({costProfile:p,targetKey:'test-device'}).choose(['cpu','gpu'],context);
  assert.equal(result.backend,i*100+1000<10100*.9?'gpu':'cpu');
 }
 first.clear();assert.equal((await second.choose(['cpu','gpu'],context)).backend,'gpu');
 const mutation={...context,features:{input_elements:1,input_bytes:4}};
 const pending=createSelector({costProfile,targetKey:'test-device'}).choose(['cpu','gpu'],mutation);
 mutation.features.input_elements=1000;mutation.features.input_bytes=4000;
 assert.equal((await pending).backend,'cpu');
});

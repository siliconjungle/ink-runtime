import test from 'node:test';
import assert from 'node:assert/strict';
import {createSelector} from '../hosts/backend-selection.mjs';
function fixture(costs,setup={}) {
  let time=0;const calls={};
  const selector=createSelector({clock:()=>time,policy:'profile'});
  const candidates=Object.entries(costs).map(([name,cost])=>({name,async:name==='gpu',prepare:()=>{time+=setup[name]??0;},run:()=>{calls[name]=(calls[name]??0)+1;time+=cost;return 7;}}));
  return {selector,candidates,calls};
}
test('chooses JS or Wasm based on complete measured cost, with cached bounded profiling',async()=>{
  for(const [costs,expected] of [[{cpu:2,javascript:0.2},'javascript'],[{cpu:0.2,javascript:2},'cpu']]) {
    const {selector,candidates,calls}=fixture(costs);
    const first=await selector.run('shape',candidates);assert.equal(first.backend,expected);assert.equal(first.result,7);
    const before={...calls};await selector.run('shape',candidates);
    for(const c of candidates)assert.equal(calls[c.name],before[c.name]+(c.name===expected?1:0));
    assert.equal(first.profile.expected_calls,32);
  }
});
test('GPU setup and transfers can outweigh a faster kernel; availability invalidates profiles',async()=>{
  const {selector,candidates}=fixture({cpu:2,javascript:0.2,gpu:0.01},{gpu:100});
  const first=await selector.run('shape',candidates);assert.equal(first.backend,'javascript');
  const changed=await selector.run('shape',candidates.slice(0,1));assert.equal(changed.backend,'cpu');
});
test('selects GPU when worthwhile, retains baseline on failure and rejects invalid configuration',async()=>{
  const {selector,candidates}=fixture({cpu:3,javascript:2,gpu:0.1});
  assert.equal((await selector.run('shape',candidates)).backend,'gpu');
  candidates[2].run=()=>{throw Error('device lost');};
  assert.equal((await selector.run('shape',candidates)).backend,'cpu');
  assert.equal((await selector.run('missing',candidates.slice(0,2),'gpu')).backend,'cpu');
  await assert.rejects(selector.run('bad',candidates,'unknown'));
  assert.throws(()=>createSelector({expectedCalls:0}));
});
test('numerical mismatch removes a profiled scalar GPU candidate; explicit JS remains available',async()=>{
  const {selector,candidates}=fixture({cpu:3,javascript:1,gpu:0.1});
  candidates[2].run=()=>8;candidates[2].verify=(v,ref)=>v===ref;
  const result=await selector.run('shape',candidates);assert.equal(result.backend,'javascript');
  assert.match(result.profile.failures.gpu,/mismatch/);
  assert.equal((await selector.run('shape',candidates,'javascript')).result,7);
});
test('charges module startup once and rechecks warm costs rather than repeatedly taxing a target',async()=>{
  const {selector,candidates}=fixture({cpu:2,javascript:0.1});candidates[1].startupMs=100;
  assert.equal((await selector.run('first',candidates)).backend,'cpu');
  const second=await selector.run('second',candidates);assert.equal(second.backend,'javascript');
  assert.equal(second.profile.setup_ms.javascript,0);
});

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
 assert.throws(()=>createSelector({costProfile,targetKey:'wrong-device'}),/target/);
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

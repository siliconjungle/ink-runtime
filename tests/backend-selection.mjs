import test from 'node:test';
import assert from 'node:assert/strict';
import {createSelector} from '../hosts/backend-selection.mjs';
function fixture(costs,setup={}) {
  let time=0;const calls={};
  const selector=createSelector({clock:()=>time});
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

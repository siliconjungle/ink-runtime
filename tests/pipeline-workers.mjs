import test from 'node:test';
import assert from 'node:assert/strict';
import {createWorkerPool} from '../hosts/pipeline-workers.mjs';
const root=new URL('https://example.test/ink/');
async function fixture(Worker,fn){const old=globalThis.Worker;globalThis.Worker=Worker;try{await fn()}finally{globalThis.Worker=old}}
test('worker timeout drains queued pure requests and removes dead capacity',async()=>fixture(class{postMessage(){}terminate(){}},async()=>{
 const pool=createWorkerPool(root,1,{timeoutMs:5}),jobs=[pool.run({},'cpu','pin'),pool.run({},'cpu','pin')];
 const results=await Promise.allSettled(jobs);assert.ok(results.every(r=>r.status==='rejected'));assert.match(results[0].reason.message,/timed out/);assert.equal(pool.available,false);assert.equal(pool.capacity,0);pool.close();
}));
test('synchronous transport errors do not strand queued jobs',async()=>fixture(class{postMessage(){throw Error('clone rejected')}terminate(){}},async()=>{
 const pool=createWorkerPool(root,1);const results=await Promise.allSettled(Array.from({length:4},()=>pool.run({},'cpu','pin')));assert.ok(results.every(r=>r.status==='rejected'));assert.equal(pool.capacity,1);pool.close();
}));
test('malformed receipts and execution errors retire only the failed worker',async()=>{
 for(const reply of [null,{id:-1},{error:'execution failed'}])await fixture(class{postMessage(data){queueMicrotask(()=>this.onmessage({data:reply?.error?{...reply,id:data.id}:reply}))}terminate(){}},async()=>{
  const pool=createWorkerPool(root,2);await assert.rejects(pool.run({},'cpu','pin'));assert.equal(pool.capacity,1);pool.close();assert.equal(pool.capacity,0);
 });
});
test('close drains active jobs, bounds pending queue, and never transfers caller buffers',async()=>fixture(class{postMessage(...args){assert.equal(args.length,1)}terminate(){}},async()=>{
 const pool=createWorkerPool(root,1);const jobs=Array.from({length:66},()=>pool.run({},'cpu','pin'));pool.close();const results=await Promise.allSettled(jobs);assert.ok(results.every(r=>r.status==='rejected'));assert.match(results[65].reason.message,/queue budget/);await assert.rejects(pool.run({},'cpu','pin'),/closed/);
}));

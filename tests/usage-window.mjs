// Imported by the adaptive suite after shared ABI assets are installed.
import test from 'node:test';import assert from 'node:assert/strict';
import{captureUsage,createUsageWindow}from'../hosts/usage-window.mjs';
test('bounded usage sampling marks missing/overflow evidence and detaches reports',async()=>{
 const r=createUsageWindow({windowCalls:4,sampleEvery:1,maxSamples:2,historyWindows:1});
 for(let i=0;i<4;i++)await r.observe(captureUsage({kind:'read',key:i}));
 assert.equal(r.snapshot().windows[0].summary.saturated,true);
 for(let i=0;i<4;i++)await r.observe(null);
 assert.equal(r.snapshot().windows.length,1);assert.equal(r.snapshot().windows[0].operation_mix_drift,null);
 const s=r.snapshot();s.windows[0].summary.samples=99;assert.equal(r.snapshot().windows[0].summary.samples,0);
 assert.equal(captureUsage({kind:'read',size:Infinity}),null);assert.equal(captureUsage({kind:'other'}),null);
 assert.throws(()=>createUsageWindow({maxSamples:257}),/budget/);
});

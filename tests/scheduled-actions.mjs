import test from 'node:test';
import assert from 'node:assert/strict';
import {ScheduledActions} from '../hosts/scheduled-actions.mjs';
const deferred=()=>{let resolve,reject;const promise=new Promise((a,b)=>{resolve=a;reject=b;});return {promise,resolve,reject};};
test('pause quiesces a live action and invalidates a pending plan',async()=>{
 const gate=deferred(),planned=deferred(),finished=deferred();let calls=0;
 const service=new ScheduledActions({host:{async invoke(){calls++;await gate.promise;return 7;}},plan:async()=>({id:'one',call:'work',args:[]}),onResult(){finished.resolve();},idleMs:60000});
 const run=service.run();await new Promise(resolve=>setTimeout(resolve,0));assert.equal(calls,1);
 let paused=false;const pause=service.pause().then(()=>paused=true);await Promise.resolve();assert.equal(paused,false);
 gate.resolve();await pause;await finished.promise;assert.equal(service.paused,true);assert.equal(calls,1);await service.stop();await run;
 const next=new ScheduledActions({host:{invoke(){calls++;}},async plan(){planned.resolve();return gate2.promise;}});
 const gate2=deferred(),run2=next.run();await planned.promise;const stopping=next.pause();gate2.resolve({id:'late',call:'work',args:[]});await stopping;await next.stop();await run2;assert.equal(calls,1);
});
test('continuous turns yield, accept wake and stop without closing the shared host',async()=>{
 let calls=0,timers=0,closed=false;const ready=deferred();let service;
 service=new ScheduledActions({host:{async invoke(){calls++;return calls;},close(){closed=true;}},plan:async()=>({id:'turn-'+calls,call:'work',args:[calls]}),onResult(){service.wake();if(calls===4)ready.resolve();}});
 const timer=setInterval(()=>timers++,1);const running=service.run();await ready.promise;await service.pause();assert.equal(calls,4);assert(timers>0);await service.stop();await running;clearInterval(timer);assert.equal(closed,false);
 service.resume();const again=service.run();await service.pause();await service.stop();await again;
});
test('abort and plan/storage failure stop explicitly; no automatic retry',async()=>{
 const abort=new AbortController();abort.abort();let calls=0;
 const service=new ScheduledActions({host:{invoke(){calls++;throw Error('uncertain publication');}},plan:()=>({id:'stable',call:'work',args:[]})});
 await service.run({signal:abort.signal});assert.equal(calls,0);
 await assert.rejects(service.run(),/uncertain publication/);assert.equal(calls,1);assert.equal(service.running,false);
 const invalid=new ScheduledActions({host:{invoke(){calls++;}},plan:()=>({id:'bad',call:'work',args:[],delayMs:0})});
 await assert.rejects(invalid.run(),/delay/);assert.equal(calls,1);
});
test('a stopped but unresolved turn cannot be overlapped by another run',async()=>{
 const gate=deferred(),planning=deferred();let calls=0;
 const service=new ScheduledActions({host:{invoke(){calls++;}},async plan(){planning.resolve();return gate.promise;}});
 const running=service.run();await planning.promise;const stopping=service.stop();assert.throws(()=>service.run(),/already running/);
 gate.resolve({id:'stale',call:'work',args:[]});await stopping;await running;assert.equal(calls,0);
});

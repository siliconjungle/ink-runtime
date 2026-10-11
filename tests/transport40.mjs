import test from 'node:test';
import assert from 'node:assert/strict';
import {transportFixture} from './transport40-fixture.mjs';
const cases=[['words',Uint32Array.of(0,4294967295,17),[0,4294967295,17]],['signed',[-2147483648,2147483647,-1],[-2147483648,2147483647,-1]],['flags',[false,true,false],[false,true,false]],['floats',[-0,1.5,{F32Bits:0x7fc12345},{F32Bits:0xff800000}],[-0,1.5,{F32Bits:0x7fc12345},{F32Bits:0xff800000}]],['wide',new BigUint64Array([0n,9007199254740992n,18446744073709551615n]),[0,'9007199254740992','18446744073709551615']],['pair',[1n,18446744073709551615n],[1,'18446744073709551615']],['record',{flag:true,wide:18446744073709551615n,pair:[-0,{F32Bits:0x7fc12345}]},{flag:true,wide:'18446744073709551615',pair:[-0,{F32Bits:0x7fc12345}]}],['scalar',4294967295,4294967295]];
test('terminal captured CPU results decode before reset across all packed carriers and memory growth',async()=>transportFixture(async(runtime,counts)=>{
 const retained=[];
 for(const [name,input,expected]of cases){const r=await runtime.call(name,[input]);assert.deepEqual(r.value,expected);retained.push([r.value,expected]);assert.strictEqual(r.values.result,r.value)}
 assert.equal(counts.arena_result_slices,0,'terminal arena output must not escape in a redundant packed clone');
 await runtime.call('words',[Uint32Array.of(99)]);for(const [actual,expected]of retained)assert.deepEqual(actual,expected);
 await runtime.close();for(const [actual,expected]of retained)assert.deepEqual(actual,expected);
}));
test('submission snapshots packed arguments before caller mutation',async()=>transportFixture(async runtime=>{
 const xs=Uint32Array.of(1,2),floats=[{F32Bits:0x7fc12345},-0],record={flag:true,wide:18446744073709551615n,pair:[-0,{F32Bits:0xff800000}]};
 const p=runtime.call('words',[xs]),q=runtime.call('floats',[floats]),r=runtime.call('record',[record]);xs.fill(9);floats[0].F32Bits=0;floats[1]=5;record.flag=false;record.wide=0n;record.pair[1].F32Bits=0;
 assert.deepEqual((await p).value,[1,2]);assert.deepEqual((await q).value,[{F32Bits:0x7fc12345},-0]);assert.deepEqual((await r).value,{flag:true,wide:'18446744073709551615',pair:[-0,{F32Bits:0xff800000}]});
}));
test('pipeline outputs own values while used packed intermediates survive resets and feedback',async()=>transportFixture(async(runtime,counts)=>{
 const p=await runtime.pipeline({steps:[{id:'a',call:'increment',args:[Uint32Array.of(0,4294967295)]},{id:'b',call:'increment',args:[{step:'a'}]},{id:'c',call:'words',args:[{step:'a'}]}],outputs:['a','b','c']},{backend:'cpu'});
 assert.deepEqual(p.values,{a:[1,0],b:[2,1],c:[1,0]});assert.equal(counts.arena_result_slices,1,'only a is consumed by later stages');
 const before=counts.arena_result_slices;
 const r=await runtime.pipeline({inputs:{xs:[4294967295,1]},steps:[{id:'next',call:'increment',args:[{input:'xs'}]}],feedback:{xs:'next'},iterations:3},{backend:'cpu'});
 assert.deepEqual(r.values.next,[2,4]);assert.equal(counts.arena_result_slices-before,2,'first two feedback values cross resets; final value decodes directly');assert.deepEqual(p.values,{a:[1,0],b:[2,1],c:[1,0]});
}));
test('CPU destination conversion writes native values once and preserves its tail',async()=>transportFixture(async(runtime,counts)=>{
 const wide=new BigUint64Array([0n,0n,77n]);assert.equal((await runtime.callInto('wide',[[9007199254740992n,18446744073709551615n]],wide,{backend:'cpu'})).count,2);assert.deepEqual(wide,new BigUint64Array([9007199254740992n,18446744073709551615n,77n]));
 const floats=[0,0,'tail'];await runtime.callInto('floats',[[-0,{F32Bits:0x7fc12345}]],floats,{backend:'cpu'});assert.ok(Object.is(floats[0],-0));assert.deepEqual(floats[1],{F32Bits:0x7fc12345});assert.equal(floats[2],'tail');
 assert.equal(counts.arena_result_slices,0);
}));

test('terminal output materialization retains arbitrary pipeline identifiers',async()=>transportFixture(async runtime=>{
 const result=await runtime.pipeline({steps:[{id:'__proto__',call:'words',args:[[1,2]]},{id:'constructor',call:'increment',args:[{step:'__proto__'}]}],outputs:['__proto__','constructor']},{backend:'cpu'});
 assert.deepEqual(Object.keys(result.values),['__proto__','constructor']);assert.deepEqual(result.values.__proto__,[1,2]);assert.deepEqual(result.values.constructor,[2,3]);
}));

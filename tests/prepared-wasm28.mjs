import test from 'node:test';
import assert from 'node:assert/strict';
import {createPackedCall} from '../hosts/prepared-wasm.mjs';

function instance() {
  const memory=new WebAssembly.Memory({initial:1});
  let top=64, allocations=0, calls=0;
  return {
    memory,
    get allocations(){return allocations},get calls(){return calls},
    ink_compute_reset(){top=64},
    ink_alloc(count,width){allocations++;const ptr=(top+7)&~7;top=ptr+Math.max(count,1)*width;const pages=Math.ceil(top/65536);if(pages>memory.buffer.byteLength/65536)memory.grow(pages-memory.buffer.byteLength/65536);return ptr},
    ink_compute_mark(){return top},ink_compute_rewind(mark){top=mark},
    ink_compute_into(index,pointers,lengths,destination){
      calls++;assert.equal(index,7);
      const table=new DataView(memory.buffer),ptr=table.getUint32(pointers,true),n=table.getUint32(lengths,true);
      // Deliberately grow memory through a literal scratch stage on each call.
      const scratch=this.ink_alloc(20000,4);
      const input=new Uint32Array(memory.buffer,ptr,n),mid=new Uint32Array(memory.buffer,scratch,n),out=new Uint32Array(memory.buffer,destination,n);
      for(let i=0;i<n;i++)mid[i]=(input[i]+1)>>>0;
      for(let i=0;i<n;i++)out[i]=Math.imul(mid[i],3)>>>0;
      return n;
    }
  };
}
test('prepared capture and destination persist across scratch rewinds and memory growth',()=>{
  const cpu=instance(),input=Uint32Array.of(0,4294967295,17);
  const call=createPackedCall(cpu,7,[{words:input,count:3}],8);
  const setup=cpu.allocations;
  assert.deepEqual(Array.from(call.run()),[3,0,54]);
  assert.deepEqual(input,Uint32Array.of(0,4294967295,17));
  for(let i=0;i<20;i++)assert.deepEqual(Array.from(call.run()),[3,0,54]);
  assert.equal(cpu.allocations-setup,21,'only the deliberate scratch producer allocates after preparation');
  call.writeArgument(0,Uint32Array.of(9,10),2);
  assert.deepEqual(Array.from(call.run()),[30,33]);
  call.writeArgument(0,new Uint32Array(),0);assert.equal(call.run().length,0);
  call.close();assert.throws(()=>call.run(),/closed/);
});
test('separate prepared instances do not invalidate each other',()=>{
  const a=createPackedCall(instance(),7,[{words:Uint32Array.of(1),count:1}],1);
  const b=createPackedCall(instance(),7,[{words:Uint32Array.of(11),count:1}],1);
  assert.deepEqual(Array.from(a.run()),[6]);assert.deepEqual(Array.from(b.run()),[36]);
  assert.deepEqual(Array.from(a.run()),[6]);a.close();b.close();
});

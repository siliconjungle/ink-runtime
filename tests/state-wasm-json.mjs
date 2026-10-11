import assert from 'node:assert/strict';
import {parseExactJSON,StatefulModule} from '../hosts/state-wasm.mjs';

assert.deepEqual(parseExactJSON(' {"a":[true,false,null,18446744073709551615,-123456789012345678901234567890,1.25,2e3,-0.0]} '),
  {a:[true,false,null,18446744073709551615n,-123456789012345678901234567890n,1.25,2000,-0]});
for(const text of ['', 'plain', 'quote"slash\\', '\u0000\n\t', 'é😀', '\ud800'])
  assert.equal(parseExactJSON(JSON.stringify(text)),text);
const special=parseExactJSON('{"__proto__":{"polluted":true},"constructor":9,"a":1,"a":2}');
assert.equal(Object.getPrototypeOf(special),Object.prototype);
assert.equal(Object.hasOwn(special,'__proto__'),true);
assert.equal(special.__proto__.polluted,true);
assert.equal(special.constructor,9n);assert.equal(special.a,2n);assert.equal({}.polluted,undefined);
for(const text of ['', '01', '+1', '1.', '1e', '1e9999', 'truefalse', '[1,]', '{"a":1,}',
  '"unterminated', '"bad\\x"', '"bad\\uZZZZ"', '"bad\n"', '{"a":1e9999,"a":0}'])
  assert.throws(()=>parseExactJSON(text),SyntaxError,text);
assert.doesNotThrow(()=>parseExactJSON('['.repeat(128)+'0'+']'.repeat(128)));
assert.throws(()=>parseExactJSON('['.repeat(129)+'0'+']'.repeat(129)),RangeError);

// The length export can grow memory, and freeing erases the guest bytes.
// Decoded responses must survive both; binary snapshots must remain owned.
const memory=new WebAssembly.Memory({initial:1});let length=0,releases=0;
const exports={memory,lang_abi_version:()=>1,lang_buffer_ptr:()=>64,
  lang_buffer_len:()=>{memory.grow(1);return length;},
  lang_buffer_free:()=>{new Uint8Array(memory.buffer,64,length).fill(0);releases++;return 1;}};
const host=new StatefulModule({exports});
function set(bytes){length=bytes.length;new Uint8Array(memory.buffer,64,length).set(bytes);}
set(new TextEncoder().encode('{"ok":{"number":18446744073709551615}}'));
assert.deepEqual(host.response(1),{number:18446744073709551615n});assert.equal(releases,1);
set(new TextEncoder().encode('{bad}'));assert.throws(()=>host.response(1),SyntaxError);assert.equal(releases,2);
set(new Uint8Array([0xff]));assert.throws(()=>host.response(1),TypeError);assert.equal(releases,3);
set(new Uint8Array([1,2,3]));const snapshot=host.take(1);
assert.deepEqual(snapshot,new Uint8Array([1,2,3]));assert.equal(releases,4);
assert.deepEqual(new Uint8Array(memory.buffer,64,3),new Uint8Array(3));
console.log('exact JSON and decoded response ownership: passed');

import fs from 'node:fs';
const module=await WebAssembly.compile(fs.readFileSync(process.argv[2]));
if(WebAssembly.Module.imports(module).length)throw Error('unexpected definition imports');
const {exports:e}=await WebAssembly.instantiate(module,{});
function read(bytes){let at=0;while(at<bytes.length){const n=fs.readSync(0,bytes,at,bytes.length-at,null);if(!n)return at===0?false:(()=>{throw Error('truncated definition frame');})();at+=n;}return true;}
const header=Buffer.alloc(8);
while(read(header)){const functionIndex=header.readUInt32LE(0),length=header.readUInt32LE(4);if(length>4*1024*1024)throw Error('definition request word limit');const bytes=Buffer.alloc(length*4);if(!read(bytes))throw Error('missing definition input');const ptr=e.ink_definition_input()>>>0;new Uint8Array(e.memory.buffer,ptr,bytes.length).set(bytes);const output=e.ink_definition_call(functionIndex,ptr,length)>>>0;const prefix=Buffer.alloc(4);prefix.writeUInt32LE(output);fs.writeSync(1,prefix);if(output){const result=e.ink_definition_result()>>>0;fs.writeSync(1,new Uint8Array(e.memory.buffer,result,output*4));}}

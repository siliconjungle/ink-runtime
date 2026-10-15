import test from 'node:test';
import assert from 'node:assert/strict';
import {mkdtemp,writeFile,rm} from 'node:fs/promises';
import {tmpdir} from 'node:os';
import {join} from 'node:path';
import {pathToFileURL} from 'node:url';
import {readArtifact} from '../hosts/initial-state.mjs';
test('initialized artifact bytes bind size/hash and reject corruption/truncation',async()=>{
 const dir=await mkdtemp(join(tmpdir(),'ink-initial-'));
 try {
  const bytes=new Uint8Array([0,127,255]),file=join(dir,'seed ?#%.bin');await writeFile(file,bytes);
  const sha256=[...new Uint8Array(await crypto.subtle.digest('SHA-256',bytes))].map(x=>x.toString(16).padStart(2,'0')).join('');
  const binding={bytes:3,sha256};assert.deepEqual(await readArtifact(pathToFileURL(file),binding),bytes);
  for(const changed of [{...binding,bytes:2},{...binding,bytes:4},{...binding,sha256:'0'.repeat(64)},{...binding,bytes:64*1024*1024+1}])await assert.rejects(readArtifact(pathToFileURL(file),changed));
  await writeFile(file,[...bytes].reverse().toString());await assert.rejects(readArtifact(pathToFileURL(file),binding));
 }finally{await rm(dir,{recursive:true,force:true});}
});

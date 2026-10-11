// Resolve shared runtime assets from the exact independently pinned ABI package.
import {execFileSync} from 'node:child_process';
import {fileURLToPath} from 'node:url';
import {copyFile} from 'node:fs/promises';
import {join} from 'node:path';
execFileSync(process.env.CARGO??'cargo',['run','--locked','--manifest-path',fileURLToPath(new URL('../Cargo.toml',import.meta.url)),'--example','host-test-assets'],{stdio:'pipe'});
export async function copySharedHostAssets(dir){
 for(const name of ['ink-abi.mjs','ink-selection.mjs','ink-selection.wasm'])
  await copyFile(new URL('../hosts/'+name,import.meta.url),join(dir,name));
}

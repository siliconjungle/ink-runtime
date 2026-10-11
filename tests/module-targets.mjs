import {test} from 'node:test';
import assert from 'node:assert/strict';
import {readFileSync,writeFileSync,mkdtempSync,mkdirSync,rmSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {join} from 'node:path';
import {execFileSync} from 'node:child_process';

test('mixed native package ignores a preserved stale pure-library target',()=>{
  const source=readFileSync(new URL('../src/module.rs',import.meta.url),'utf8');
  const literal=source.match(/super::put\(dir,"native\/Cargo.toml",("(?:[^"\\]|\\.)*")\)/)[1];
  // Use the actual generated package section. Dependencies do not influence
  // target discovery, and omitting them keeps this regression offline.
  const manifest=JSON.parse(literal).split('[dependencies]')[0]+'[workspace]\n';
  const root=mkdtempSync(join(tmpdir(),'ink-module-targets-'));
  try{
    mkdirSync(join(root,'src'));
    const stale='compile_error!("a stale packed library must not be built");\n';
    writeFileSync(join(root,'src/lib.rs'),stale);
    writeFileSync(join(root,'src/main.rs'),'fn main() {}\n');
    writeFileSync(join(root,'Cargo.toml'),manifest);
    const metadata=()=>JSON.parse(execFileSync('cargo',['metadata','--no-deps','--offline','--format-version','1','--manifest-path',join(root,'Cargo.toml')],{encoding:'utf8'}));
    assert.deepEqual(metadata().packages[0].targets.map(t=>t.kind),[['bin']]);
    execFileSync('cargo',['check','--offline','--manifest-path',join(root,'Cargo.toml')],{stdio:'pipe'});
    assert.equal(readFileSync(join(root,'src/lib.rs'),'utf8'),stale);
    writeFileSync(join(root,'Cargo.toml'),manifest.replace('autolib=false\n',''));
    assert.equal(metadata().packages[0].targets.some(t=>t.kind.includes('lib')),true);
    assert.throws(()=>execFileSync('cargo',['check','--offline','--manifest-path',join(root,'Cargo.toml')],{stdio:'pipe'}),/Command failed/);
  }finally{rmSync(root,{recursive:true,force:true});}
});

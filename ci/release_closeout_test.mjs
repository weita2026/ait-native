#!/usr/bin/env node
import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { mkdtempSync, mkdirSync, writeFileSync, readFileSync, existsSync, rmSync } from 'node:fs';
import { spawnSync } from 'node:child_process';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
// Exercise the expression used by the PowerShell workflow, including its
// case-sensitive operator and strict end-of-input behavior.
const workflow=readFileSync(path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../.github/workflows/ait-release-winget-discovery.yml'), 'utf8');
const guard=workflow.match(/\$env:RELEASE_ID\s+(-cnotmatch)\s+'([^']+)'/);
assert.ok(guard, 'discovery must use a case-sensitive release-family guard');
const releaseFamilyPattern=new RegExp(guard[2]);
for (const id of ['REL-FAM-79B1908F60843FE4', 'REL-FAM-0123456789ABCDEF', 'REL-FAM-0000000000000000']) {
 assert.equal(releaseFamilyPattern.test(id), true, `valid family: ${JSON.stringify(id)}`);
}
for (const id of ['', 'REL-FAM1234', 'REL-0123456789ABCDEF', 'REL-FAM-0123456789ABCDE', 'REL-FAM-0123456789ABCDEF0', 'REL-FAM-0123456789ABCDEG', 'REL-FAM-0123456789abcdef', 'rel-fam-0123456789ABCDEF', ' REL-FAM-0123456789ABCDEF', 'REL-FAM-0123456789ABCDEF ', 'REL-FAM-0123456789ABCDEF\n', 'REL-FAM-0123456789ABCDEF\r\n', 'REL-FAM-0123456789ABCDEF\u2028', 'REL-FAM-0123456789ABCDEF\u0000']) {
 assert.equal(releaseFamilyPattern.test(id), false, `invalid family: ${JSON.stringify(id)}`);
}
const root=mkdtempSync(path.join(os.tmpdir(),'ait-closeout-test-'));
const write=(name,obj)=>{const f=path.join(root,name);mkdirSync(path.dirname(f),{recursive:true});writeFileSync(f,JSON.stringify(obj)+'\n');};
const sha=name=>createHash('sha256').update(readFileSync(path.join(root,name))).digest('hex');
const run=()=>spawnSync(process.execPath,[path.join(path.dirname(fileURLToPath(import.meta.url)),'release_closeout.mjs'),'--records-root',root,'--version','1.1.4','--output',path.join(root,'release-closeout.json')],{encoding:'utf8'});
try{
 const release={id:'REL-FAM-0123456789ABCDEF',version:'1.1.4'};
 write('candidate.json',{release,status:'ready_for_immutable_tag'});
 write('web-admission/web-admission.json',{release_id:release.id,status:'admitted'});
 write('public-tag/receipt.json',{kind:'tag',status:'complete'});
 write('endpoints.json',{release});write('operator-status.json',{release,status:'published_readback_complete'});
 write('winget-submission.json',{contract:'ait.release.winget-submission/v1',version:'1.1.4',status:'submitted',pull_request:{number:42}});
 write('latest-alias-cache/cache.json',{contract:'ait.release.artifact-cache/v1',status:'complete'});
 assert.notEqual(run().status,0);assert.equal(existsSync(path.join(root,'release-closeout.json')),false);
 const merge={contract:'ait.release.winget-status/v1',version:'1.1.4',status:'submitted',submission_sha256:sha('winget-submission.json'),pull_request:{number:42,merge_commit_sha:'a'.repeat(40)}};
 const discovery={contract:'ait.release.winget-discovery/v1',version:'1.1.4',release_id:release.id,status:'pending',submission_sha256:merge.submission_sha256,merge_commit_sha:'a'.repeat(40)};
 write('winget-status.json',merge);write('winget-discovery-cache/payload/winget-discovery.json',discovery);
 assert.notEqual(run().status,0);
 merge.status='merged';write('winget-status.json',merge);assert.notEqual(run().status,0);
 discovery.status='discoverable';discovery.version='1.1.3';write('winget-discovery-cache/payload/winget-discovery.json',discovery);assert.notEqual(run().status,0);
 discovery.version='1.1.4';write('winget-discovery-cache/payload/winget-discovery.json',discovery);assert.notEqual(run().status,0);
 const installation={...discovery,contract:'ait.release.winget-installation/v1',status:'installed_verified',source:'winget',package_id:'Weita.AitNative',architecture:'x64',scope:'user',fresh_host:true,commands:['ait','ait-server','ait-runner'].map(name=>({name,reported_version:`${name} 1.1.4`,sha256:'b'.repeat(64)}))};
 for(const [key,value] of Object.entries({status:'pending',version:'1.1.3',release_id:'wrong-family',submission_sha256:'0'.repeat(64),merge_commit_sha:'0'.repeat(40),source:'local',architecture:'arm64',fresh_host:false,commands:[]})) {
   write('winget-discovery-cache/payload/winget-installation.json',{...installation,[key]:value});assert.notEqual(run().status,0,key);
 }
 write('winget-discovery-cache/payload/winget-installation.json',installation);assert.equal(run().status,0);
 assert.equal(JSON.parse(readFileSync(path.join(root,'release-closeout.json'))).status,'published');
 assert.equal(run().status,73);console.log('closeout pending/merge/discovery tests passed');
}finally{rmSync(root,{recursive:true,force:true});}

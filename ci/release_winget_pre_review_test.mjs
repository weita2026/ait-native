import assert from 'node:assert/strict';
import { validatePreReviewReceipt, verifyPreReview } from './release_winget_pre_review_verify.mjs';
const expected={version:'1.1.4',release_id:'REL-FAM-79B1908F60843FE4',architecture:'x64',control_commit:'a'.repeat(40),bootstrap_sha256:'b'.repeat(64),files:['Weita.AitNative.yaml','Weita.AitNative.installer.yaml','Weita.AitNative.locale.en-US.yaml'].map(name=>({name,sha256:'c'.repeat(64)}))};
const receipt={contract:'ait.release.winget-pre-review/v1',status:'installed_verified',...expected,fresh_host:true,scope:'user',manifests:Object.fromEntries(expected.files.map(x=>[x.name,x.sha256])),commands:['ait','ait-server','ait-runner'].map(name=>({name,reported_version:`${name} 1.1.4`,sha256:'d'.repeat(64)}))};
validatePreReviewReceipt(receipt,expected);
for(const [key,value] of Object.entries({status:'pending',version:'1.1.3',release_id:'REL-FAM-0000000000000000',architecture:'arm64',control_commit:'0'.repeat(40),bootstrap_sha256:'0'.repeat(64),fresh_host:false,scope:'machine',commands:[],manifests:{}})) {
 assert.throws(()=>validatePreReviewReceipt({...receipt,[key]:value},expected),undefined,key);
}
assert.throws(()=>verifyPreReview({}),/pre-review-run-id/);
assert.throws(()=>verifyPreReview({runId:'123',releaseId:'REL-FAM-79B1908F60843FE4\n'}),/release-id/);
console.log('WinGet pre-review binding and fail-closed tests passed');

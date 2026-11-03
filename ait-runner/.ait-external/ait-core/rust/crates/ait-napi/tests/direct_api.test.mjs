import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { test } from 'node:test';
import { Worker } from 'node:worker_threads';
import { once } from 'node:events';
import { spawnSync } from 'node:child_process';
import { createHash } from 'node:crypto';

const addonPath = process.env.AIT_NAPI_TEST_ADDON;
assert.ok(addonPath && path.isAbsolute(addonPath), 'Set AIT_NAPI_TEST_ADDON to the actual compiled addon');
const nativeModule = { exports: {} };
process.dlopen(nativeModule, addonPath);
const binding = nativeModule.exports;

test('Snapshot authoring scalar grammar is shared without repository discovery', () => {
  for(const value of ['LT-0001','LCT-1159','LT-0001/C-01']) assert.equal(binding.parseSnapshotAuthoringReference('  '+value+'  '),value);
  for(const value of ['', 'C-01', 'SNP-123456789ABC', 'invalid']) assert.throws(()=>binding.parseSnapshotAuthoringReference(value),error=>error.message==='Snapshot creation requires a Task ID.');
});

test('remote-head recovery preview uses remote ancestry without opening damaged local authority', async () => {
  const worker=new Worker(`
    const {parentPort}=require('node:worker_threads'),http=require('node:http');let calls=[];
    const server=http.createServer((req,res)=>{
      calls.push([req.method,req.url]);res.setHeader('content-type','application/json');
      if(req.url.endsWith('/lines/main')) res.end(JSON.stringify({repo_name:'fixture',line_name:'main',head_snapshot_id:'SNP-HEAD'}));
      else if(req.url.includes('/remote-sync/zstd-bulk/import-manifests/')) {
        const id=req.url.includes('SNP-ROOT')?'SNP-ROOT':'SNP-HEAD',parents=id==='SNP-HEAD'?['SNP-ROOT']:[];
        res.end(JSON.stringify({contract:'ait.remote_sync.zstd_bulk.import_manifest.v1',repo_name:'fixture',snapshot_id:id,
          snapshots:[{snapshot_id:id,parent_snapshot_ids:parents,primary_parent_snapshot_id:parents[0]??null,parent_snapshot_id:parents[0]??null,root_tree_pack_id:'TPK-FIXTURE',root_entry_ordinal:0,manifest_hash:'manifest',snapshot_kind:'workspace',file_count:0,total_bytes:0,created_at:'2026-10-26T00:00:00Z'}],
          object_packs:[],tree_packs:[{pack_id:'TPK-FIXTURE',pack_format:'ait-tree-pack-v2-zstd-chunked',tree_count:1,total_bytes:0,pack_index_entry_name:'index',pack_index_checksum:'checksum',created_at:'2026-10-26T00:00:00Z'}],blob_locators:[],tree_locators:[{tree_id:'TRE-FIXTURE',entry_count:0,tree_pack_id:'TPK-FIXTURE',tree_pack_checksum:'checksum',created_at:'2026-10-26T00:00:00Z'}],line_update:null}));
      } else {res.statusCode=404;res.end(JSON.stringify({detail:'unexpected fixture request'}));}
    });server.listen(0,'127.0.0.1',()=>parentPort.postMessage(server.address().port));
    parentPort.on('message',()=>parentPort.postMessage(calls));
  `,{eval:true});
  const fixture=fs.realpathSync(fs.mkdtempSync(path.join(os.tmpdir(),'ait-head-recovery-'))),root=path.join(fixture,'fixture');fs.mkdirSync(root);
  try {
    const [port]=await once(worker,'message');fs.mkdirSync(path.join(root,'.ait'));
    fs.writeFileSync(path.join(root,'.ait','config.json'),JSON.stringify({repo_name:'fixture',repository_index:7,default_line:'main',default_remote:'origin',remotes:{origin:{url:'http://127.0.0.1:'+port,repo_name:'fixture'}}}));
    const authority=path.join(root,'.ait','binary-db');fs.writeFileSync(authority,'deliberately damaged authority');
    const preview=binding.remoteRecoverHead(root);
    assert.equal(preview.apply,false);assert.equal(preview.snapshot_id,'SNP-HEAD');assert.equal(preview.history_mode,'complete_ancestry');
    assert.equal(preview.reachable_snapshot_count,2);assert.equal(preview.object_pack_count,0);assert.equal(preview.tree_pack_count,1);
    assert.deepEqual(preview.source_parent_snapshot_ids,['SNP-ROOT']);
    assert.match(binding.formatRemoteRecoverHead(preview),/^ait remote recover-head\n/);
    assert.equal(fs.readFileSync(authority,'utf8'),'deliberately damaged authority');
    for(const jobs of [0,65,-1,18446744073709551615n]) assert.throws(()=>binding.remoteRecoverHead(root,{jobs}),/jobs.*1 through 64/);
    assert.throws(()=>binding.remoteRecoverHead(root,{remoteName:'absent'}),/absent/);
    worker.postMessage('calls');const [calls]=await once(worker,'message');
    assert.equal(calls.length,3);assert.ok(calls.every(([method])=>method==='GET'));
  } finally {await worker.terminate();fs.rmSync(fixture,{recursive:true,force:true});}
});

test('every frozen native command has a concrete addon operation or the sole maintenance exception', () => {
  const inventory=JSON.parse(fs.readFileSync(new URL('../../ait-cli/tests/fixtures/language_cli_inventory.json',import.meta.url)));
  const mapping=JSON.parse(fs.readFileSync(new URL('./cli_bindings.json',import.meta.url)));
  const paths=inventory.commands.map(row=>row.path);
  const leaves=inventory.commands.filter(row=>!paths.some(p=>p.length>row.path.length && row.path.every((value,i)=>p[i]===value))).map(row=>row.path.slice(1).join(' '));
  assert.deepEqual(mapping.commands.map(row=>row.command).sort(),leaves.sort());
  for(const row of mapping.commands) {
    if(row.classification==='native_maintenance_only') {
      assert.equal(row.command,'current-source-cache run-cli');assert.equal(row.operation,null);
    } else {assert.equal(row.classification,'concrete_binding');assert.equal(typeof binding[row.operation],'function',row.command+': '+row.operation);}
  }
  assert.equal(mapping.commands.filter(row=>row.classification==='native_maintenance_only').length,1);
  assert.equal(binding.runCli,undefined);assert.equal(binding.taskWorkflowRepoCommand,undefined);
});

test('bounded Binary DB maintenance bindings preserve source rejection and native text', () => {
  const root=fs.realpathSync(fs.mkdtempSync(path.join(os.tmpdir(),'ait-binary-db-bindings-'))),generation=path.join(root,'generation');
  try {
    for(const selector of ['', 'u32', 'u64-second-v0']) {
      assert.throws(()=>binding.binaryDbUpgradeU64Seconds(root,generation,selector,true),/expected exact selector "u32-time-v0"/);
      assert.equal(fs.existsSync(generation),false);
    }
    fs.mkdirSync(generation);
    assert.throws(()=>binding.binaryDbUpgradeU64Seconds(root,generation,'u32-time-v0'),/output root must not already exist/);
    assert.throws(()=>binding.binaryDbAgentPool50(root,generation,true),/restricted to ait-core/);
    assert.deepEqual(fs.readdirSync(generation),[]);
    assert.equal(binding.formatBinaryDbAgentPool50({status:'already_converted',activated:true}),'ait binary-db agent-pool50\nstatus: already converted');
    assert.equal(binding.formatBinaryDbAgentPool50({status:'staged_retry',activated:false}),'ait binary-db agent-pool50\nstatus: staged retry\nactivated: false');
    assert.equal(binding.formatBinaryDbAgentPool50({generation_root:'out',activated:true,retained_previous_authority:'before'}),'ait binary-db agent-pool50\ngeneration root: out\nactivated: true\nretained previous authority: before');
    assert.equal(binding.formatBinaryDbUpgradeU64Seconds({repo_name:'fixture',source_time_width:'u32-time-v0',target_time_width:'u64-second-v0',generation_root:'out',source_bytes:18446744073709551615n,target_bytes:42,rebuilt_index_file_count:2,activated:false,retained_previous_authority:null}),
      'ait binary-db upgrade-u64-seconds\nrepository: fixture\nsource time width: u32-time-v0\ntarget time width: u64-second-v0\ngeneration root: out\nsource bytes: 18446744073709551615\ntarget bytes: 42\nrebuilt content indexes: 2\nactivated: false');
  } finally {fs.rmSync(root,{recursive:true,force:true});}
});

test('actual addon shares External selection, diagnostics, links and Team status', () => {
  const fixture=fs.realpathSync(fs.mkdtempSync(path.join(os.tmpdir(),'ait-external-team-'))),root=path.join(fixture,'repo'),linked=path.join(fixture,'linked');
  fs.mkdirSync(root);fs.mkdirSync(linked);
  try {
    binding.taskWorkflowInit(root);binding.configureRepository(root,{agent_teams:'disabled'});
    const status=binding.externalStatus(root);
    assert.deepEqual(status.externals,[]);assert.match(binding.formatExternalStatus(status),/^ait external status/);
    const diagnostic=binding.externalDoctor(root,true);
    assert.equal(diagnostic.exitCode,0);assert.equal(diagnostic.payload.release_ready,true);
    assert.match(binding.formatExternalDoctor(diagnostic.payload),/release_ready: true/);
    const update=binding.externalUpdate(root);
    assert.equal(update.lockfile_changed,true);assert.equal(update.manifest_changed,false);
    assert.match(binding.formatExternalUpdate(update),/^ait external update/);
    const locked=binding.externalUpdate(root,{locked:true,validate:true,noRecursive:true});
    assert.equal(locked.locked,true);assert.equal(locked.recursive,false);assert.equal(locked.validated,true);
    for(const [options,message] of [
      [{snapshot:'S',latest:true},/either.*--to/],
      [{locked:true,name:'dep'},/does not accept/],
      [{snapshot:'S'},/requires an external name/],
      [{latest:true},/requires an external name/],
      [{name:'dep'},/requires.*--to/],
    ]) assert.throws(()=>binding.externalUpdate(root,options),message);
    fs.rmSync(path.join(root,'ait-external.lock'));
    fs.writeFileSync(path.join(root,'ait-external.toml'),'[[external]]\nname = "dep"\nrepo_name = "dependency"\nrepository_index = 1\nremote = "origin"\nline = "main"\nsnapshot = "SNP-MISSING"\nmaterialize_to = ".ait-external/dep"\nlicense = "Apache-2.0"\n');
    const blocked=binding.externalDoctor(root,true),advisory=binding.externalDoctor(root,false);
    assert.equal(blocked.exitCode,2);assert.equal(blocked.payload.release_ready,false);
    assert.equal(advisory.exitCode,0);assert.deepEqual(blocked.payload,advisory.payload);
    assert.throws(()=>binding.externalLink(root,'dep',root),/resolves to this repository/);
    const link=binding.externalLink(root,'dep',linked);
    assert.equal(link.state,'linked');assert.equal(link.changed,true);
    assert.match(binding.formatExternalLink(link),/dep/);
    const unlink=binding.externalUnlink(root,'dep');
    assert.equal(unlink.state,'unlinked');assert.equal(unlink.changed,true);assert.equal(unlink.restore_state,'skipped_no_lockfile');
    assert.match(binding.formatExternalUnlink(unlink),/dep/);
    assert.equal(binding.externalUnlink(root,'dep').changed,false);
    const contacts=binding.teamContacts(root);
    assert.equal(contacts.exitCode,0);assert.equal(contacts.payload.team_enabled,false);assert.deepEqual(contacts.payload.members,[]);
    assert.equal(binding.formatTeamContacts(contacts.payload),'Agent Teams disabled.');
    const ensured=binding.teamSessionEnsure(root,1),acknowledged=binding.teamSessionAcknowledge(root,'invalid');
    for(const result of [ensured,acknowledged]) {assert.equal(result.exitCode,1);assert.equal(result.payload.work_started,false);assert.equal(result.payload.detail,'Agent Teams is disabled');}
    assert.equal(binding.formatTeamSessionEnsure(ensured.payload,1),'Team session: "pending"\nAgent: 1\nSession ID: unbound\nAction: none\nAgent Teams is disabled');
    assert.match(binding.formatTeamSessionAcknowledge(acknowledged.payload),/^Team activation: "pending"\nSession ID: /);
    assert.throws(()=>binding.teamSessionEnsure(root,4294967296n),/agent.*u32/);
    assert.throws(()=>binding.teamSessionEnsure(root,1,-1),/role.*u32/);
    assert.throws(()=>binding.formatTeamSessionEnsure({},4294967296n),/agent.*u32/);
  } finally {fs.rmSync(fixture,{recursive:true,force:true});}
});

test('actual addon calls active release services and preserves family routing', () => {
  const fixture=fs.realpathSync(fs.mkdtempSync(path.join(os.tmpdir(),'ait-release-operations-')));
  const source=path.join(fixture,'source'),root=path.join(fixture,'repo');
  fs.mkdirSync(source);fs.mkdirSync(root);
  const git=(...args)=>{
    const result=spawnSync('git',args,{cwd:source,encoding:'utf8'});
    assert.equal(result.status,0,result.stderr);
  };
  try {
    const target='x86_64-unknown-linux-gnu';
    const manifest={schema:'ait.release.family/v3',family:{name:'ait-native',version:'1.0.0-rc.1',channel:'rc',tag:'v1.0.0-rc.1'},targets:[target],
      public_source:{model:'release-monorepo',identity:'weita2026/ait-native',product_document:'docs/distribution.md',family_manifest:'ait-release-family.json',mapping_manifest:'ait-monorepo-source.json',
        build_entrypoints:{unix:'build-release.sh',windows:'build-release.ps1',implementation:'build-release.mjs'},subtrees:[{source_repository:'ait-core',path:'ait-core',transforms:[]}],transforms:[]},
      components:[{id:'ait',source_repository:'ait-core',source_snapshot:'SNP-111111111111',ecosystem:'native',license:'Apache-2.0',version_scheme:'family',version:'1.0.0-rc.1',artifacts:[{kind:'native-executable',targets:[target]}]}],
      distributions:[{channel:'github',role:'product',identity:'weita2026/ait-native',components:['ait'],targets:[target]}],compatibility:{}};
    const outputName=process.platform==='win32'?'component-bin.exe':'component-bin';
    const adapter={schema:'ait.release.adapter/v1',package:{name:'adapter-fixture',version:'1.2.3',license_files:[{path:'LICENSE',role:'license'},{path:'NOTICE',role:'notice'}]},
      components:[{id:'component',ecosystem:'rust-fixture',working_directory:'.',dependency_files:['component.rs'],
        commands:{test:[['rustc','--version']],build:[['rustc','component.rs','-o',outputName]],smoke:[[process.platform==='win32'?'.\\component-bin.exe':'./component-bin']]},
        artifacts:[{path:outputName,kind:'native-test-binary'}]}]};
    fs.writeFileSync(path.join(source,'ait-release-family.json'),JSON.stringify(manifest));
    fs.writeFileSync(path.join(source,'ait-release.json'),JSON.stringify(adapter));
    fs.writeFileSync(path.join(source,'LICENSE'),'fixture license\n');fs.writeFileSync(path.join(source,'NOTICE'),'fixture notice\n');
    fs.writeFileSync(path.join(source,'component.rs'),'fn main() { println!("adapter smoke ok"); }\n');
    git('init','-b','main');git('config','user.name','Release Fixture');git('config','user.email','fixture@example.test');git('add','.');git('commit','-m','release source');
    binding.taskWorkflowInit(root);
    binding.gitImport(root,source,{allBranchesAndTags:true});
    const checked=binding.releaseAdapterCheck(root,'1.2.3');
    assert.equal(checked.contract,'ait.release.adapter.receipt/v1');
    assert.equal(checked.check_summary.blocking,0);
    const built=binding.releaseAdapterBuild(root,'1.2.3');
    assert.equal(built.contract,'ait.release.adapter.receipt/v1');
    assert.equal(built.check_summary.blocking,0);
    assert.throws(()=>binding.releaseCandidateCreate(root,'1.0.0-rc.1',{channel:'stable'}),/does not match/);
    const candidate=binding.releaseCandidateCreate(root,'1.0.0-rc.1',{channel:'rc'});
    assert.equal(candidate.profile,'family');assert.equal(candidate.authority.persistence,'portable_dist_dossier');
    assert.equal(candidate.authority.binary_db_layout_changed,false);
    const id=candidate.release_id;
    assert.equal(binding.releaseShow(root,id).release_id,id);
    assert.equal(binding.releaseCandidateCreate(root,'1.0.0-rc.1',{profile:'family',channel:'rc'}).release_id,id);
    assert.match(binding.formatRelease(candidate),new RegExp('^ait release '+id+'\\n'));
    for(const operation of [()=>binding.releaseCheck(root,id),()=>binding.releaseBuild(root,id)]) assert.throws(operation,/requires --receipts/);
    assert.throws(()=>binding.releaseCheck(root,id,{testsCommand:'never execute'}),/immutable component receipts/);
    assert.throws(()=>binding.releaseBuild(root,id,{nativeMatrixDir:fixture}),/component receipts/);
    assert.throws(()=>binding.releaseShow(root,id,{remoteName:'absent'}),/local portable evidence/);
    assert.throws(()=>binding.releasePublish(root,id),/protected|promot|publish/i);
    assert.throws(()=>binding.releasePackage(root,id,'pypi'),/Frozen family release is unavailable/);
    assert.throws(()=>binding.releasePromote(root,id,'rc'),/Frozen family release is unavailable/);
    const unsupported=/Local workflow event\/release\/ownership authority is unsupported/;
    assert.throws(()=>binding.releaseNativeBundle(root,'absent',fixture),unsupported);
    assert.throws(()=>binding.releaseNativeSource(root,{releaseId:'absent',target,sourceDir:fixture,runner:'fixture',runnerImage:'fixture',rustToolchain:'fixture',rustcPath:'rustc'}),unsupported);
    assert.throws(()=>binding.releaseFormula(root,'absent','fixture'),unsupported);
    assert.throws(()=>binding.releaseCandidateCreate(root,'1.0.0-rc.1',{publicSourceRoot:fixture}),/requires --profile family/);
    for(const operation of [()=>binding.releaseCheck(root,'absent',{receipts:fixture}),()=>binding.releaseBuild(root,'absent',{publicSourceRoot:fixture})]) assert.throws(operation,/applies only to a family/);
  } finally {fs.rmSync(fixture,{recursive:true,force:true});}
});

test('release formatter retains native source and family text byte boundaries', () => {
  assert.equal(binding.formatRelease({command:'release native-source',release_id:'R',status:'recorded',version:'1',snapshot_id:'S',profile:'native',target:'T',descriptor_path:'descriptor',descriptor_sha256:'sha'}),
    'ait release native-source R\nstatus: recorded\nversion: 1\nsnapshot: S\nprofile: native\ntarget: T\ndescriptor: descriptor\ndescriptor sha256: sha');
  assert.equal(binding.formatRelease({release_id:'R',status:'ready_for_protected_ci',version:'1',line:'main',snapshot_id:'S',profile:'family',channel:'rc',family:{components:[{}],targets:['T']},checks:[{check_id:'C',blocking:true,status:'fail',details:'D'}],check_summary:{total:1,blocking:1,decision:'fail'},artifacts:[{kind:'wheel',path:'a.whl'}],next_action:{detail:'next step'},formula:{path:'f.rb'}}),
    'ait release R\nstatus: ready_for_protected_ci\nversion: 1\nline: main\nsnapshot: S\nprofile: family\nchecks: fail (1 blocking / 1 recorded)\nartifacts: wheel:a.whl\nfamily channel: rc; components: 1; targets: 1\npromotion: protected CI handoff only; registry writes: no\nnext: next step\n\nrelease check detail\n- C [fail]: D\nformula draft: f.rb');
});

test('actual addon preserves retained release SDK validation and authority guards', () => {
  const root=fs.realpathSync(fs.mkdtempSync(path.join(os.tmpdir(),'ait-release-sdk-')));
  try {
    binding.taskWorkflowInit(root);
    const input={releaseId:'REL-SDK',repoName:'fixture',version:'1.2.0',lineName:'main',snapshotId:'SNP-FIXTURE',manifestHash:'fixture-hash',profile:'local-cli',metadataJson:'{ "n": 18446744073709551615 }'};
    const unsupported='Local workflow event/release/ownership authority is unsupported; Task and Change use the explicit local Binary DB authority.';
    for(const operation of [
      ()=>binding.listWorkflowReleases(root),()=>binding.getWorkflowRelease(root,'absent'),
      ()=>binding.createWorkflowRelease(root,input),
      ()=>binding.updateWorkflowRelease(root,'REL-SDK',{metadataJson:'{'}),
      ()=>binding.updateWorkflowRelease(root,'REL-SDK',{metadataSet:true}),
      ()=>binding.taskWorkflowReleaseCheck(root,'absent'),
      ()=>binding.taskWorkflowReleaseBuild(root,'absent'),
      ()=>binding.taskWorkflowReleaseFormula(root,'absent','fixture'),
      ()=>binding.taskWorkflowReleaseShow(root,'absent'),
      ()=>binding.taskWorkflowReleasePublish(root,'absent'),
    ]) assert.throws(operation,error=>error.message===unsupported);
    assert.throws(()=>binding.createWorkflowRelease(root,{...input,metadataJson:'{'}),/metadata_json/);
    assert.throws(()=>binding.updateWorkflowRelease(root,'REL-SDK',{metadataSet:true,metadataJson:'{'}),/metadata_json/);
    assert.throws(()=>binding.updateWorkflowRelease(root,'REL-SDK',{formulaSet:true,formulaJson:'{'}),/formula_json/);
    assert.throws(()=>binding.taskWorkflowReleaseCandidateCreate(root,'1.2.0','main','unknown-profile'),/profile/);
    assert.throws(()=>binding.taskWorkflowReleaseCandidateCreate(root,'1.2.0','main','local-cli'),/head snapshot/);
    assert.throws(()=>binding.taskWorkflowReleaseShow(root,'absent','absent'),/absent/);
  } finally {fs.rmSync(root,{recursive:true,force:true});}
});

test('actual addon shares context-aware workflow projections with retained PyO3', () => {
  const cases=[{"name":"project_workflow_landed_read_model","context":{},"args":[{"task":{"task_id":"RT-1","status":"active"},"change":{},"patchset":{"patchset_id":"RT-1/P-01","revision_snapshot_id":"SNP-REV"},"target_line":"main","base_line_name":"main"}],"result":{"attestation":null,"base_line":{"head_snapshot_id":null,"line_name":"main"},"change":null,"freshness":{"base_is_fresh":true,"patchset_base_snapshot_id":null,"patchset_revision_snapshot_id":"SNP-REV","preflight_state":"not_required","rebase_state":"idle","recovery_required":false,"remote_base_snapshot_id":null,"worktree_needs_retarget":false},"landing_summary":null,"next_action":{"code":"workflow_ready","command":"ait workflow ready RT-1 --apply","detail":"The workspace still has unsaved changes.","summary":"Run workflow ready before review or finish."},"patchset":{"patchset_id":"RT-1/P-01","revision_snapshot_id":"SNP-REV"},"patchset_source":null,"policy":{"checks":[],"decision":"pass","status_source":"landed_fast_path"},"review":{"approvals":1,"blocking":0,"current_patchset_id":"RT-1/P-01","reviews":[],"task_approvals":1,"team_approvals":0},"steps":[{"code":"snapshot","detail":"No additional authoring snapshot is required because the change is already applied.","label":"Snapshot","status":"done"},{"code":"patchset","detail":"Patchset `RT-1/P-01` is already part of the accepted history for this change.","label":"Patchset","status":"done"},{"code":"attestation","detail":"Finish already succeeded, so attestation gating has already cleared.","label":"Attestation","status":"done"},{"code":"review","detail":"Finish already succeeded, so review requirements were already satisfied.","label":"Review","status":"done"},{"code":"policy","detail":"Finish already succeeded, so policy has already cleared.","label":"Policy","status":"done"},{"code":"land","detail":"Task `RT-1` already has accepted work applied to `main`.","label":"Finish","status":"done"}],"suggested_commands":["ait workflow ready RT-1 --apply"],"task":{"status":"active","task_id":"RT-1"},"workspace":{"changed_count":0,"clean":false,"current_line":"unknown","head_snapshot_id":null,"workspace_matches_patchset":true,"workspace_status":"dirty"}}},{"name":"project_workflow_land_full_read_model","context":{},"args":[{"task":{"task_id":"RT-1","status":"active"},"change":{},"patchset":{"patchset_id":"RT-1/P-01","revision_snapshot_id":"SNP-REV"},"target_line":"main","base_line_name":"main"},false],"result":{"attestation":null,"base_line":{"head_snapshot_id":null,"line_name":"main"},"change":null,"freshness":{"base_is_fresh":false,"patchset_base_snapshot_id":null,"patchset_revision_snapshot_id":null,"preflight_state":"stale","rebase_state":"idle","recovery_required":true,"remote_base_snapshot_id":null,"worktree_needs_retarget":false},"landing_summary":null,"next_action":{"code":"workflow_ready","command":"ait workflow ready RT-1 --apply","detail":"The workspace still has unsaved changes.","summary":"Run workflow ready before review or finish."},"patchset":{"patchset_id":"RT-1/P-01","revision_snapshot_id":"SNP-REV"},"patchset_ci_status":null,"patchset_refresh":null,"patchset_source":null,"policy":null,"review":{"approvals":0,"blocking":0,"task_approvals":0,"team_approvals":0},"steps":[{"code":"snapshot","command":"ait snapshot create RT-1 --message \"reviewable snapshot\"","detail":"Workspace changes are still dirty, so publishable finish state needs a fresh snapshot first.","label":"Snapshot","status":"pending"},{"code":"patchset","command":"ait patchset publish RT-1 --summary \"review summary\"","detail":"The base line `main` moved to `` after patchset `RT-1/P-01` was published.","label":"Patchset","status":"stale"},{"code":"attestation","command":"ait attest put RT-1/P-01 --tests pass","detail":"No attestation is recorded yet for patchset `RT-1/P-01`. Run workflow ready so it completes Patchset CI and records the compact Attestation gate statement.","label":"Attestation","status":"pending"},{"code":"review","command":"ait config set --user-name \"<name>\"","detail":"Code review: not required; Task review: pending; Team review: unavailable while Agent Teams is disabled. Supply the acting Reviewer's own name with `--reviewer` when recording automatic approval.","label":"Review","status":"pending"},{"code":"policy","detail":"Policy is folded into `task finish` for Patchset `RT-1/P-01`; finish preflight will run the authoritative evaluation.","label":"Policy","status":"advisory"},{"code":"land","command":"ait task finish RT-1","detail":"Finish waits for review to clear first.","label":"Finish","status":"waiting"}],"suggested_commands":["ait workflow ready RT-1 --apply"],"task":{"status":"active","task_id":"RT-1"},"task_review":{"source":"built_in","value":false},"workspace":{"changed_count":0,"clean":false,"current_line":"unknown","head_snapshot_id":null,"workspace_matches_patchset":null,"workspace_status":"dirty"},"worktree_retarget":null}},{"name":"project_workflow_ready_read_model","context":{},"args":[{"task":{"task_id":"RT-1","status":"active"},"change":{},"patchset":{"patchset_id":"RT-1/P-01","revision_snapshot_id":"SNP-REV"},"target_line":"main","base_line_name":"main"},false,false,false],"result":{"base_line_name":"main","change":null,"next_action":{"code":"snapshot_create","command":"ait snapshot create RT-1 --message \"reviewable snapshot\"","detail":"The workspace still has unsaved changes.","summary":"Capture a fresh snapshot before publishing or refreshing the patchset."},"patchset":{"patchset_id":"RT-1/P-01","revision_snapshot_id":"SNP-REV"},"steps":[{"code":"snapshot","command":"ait snapshot create RT-1 --message \"reviewable snapshot\"","detail":"Workspace changes are still dirty, so ready state needs a fresh snapshot first.","label":"Snapshot","status":"pending"},{"code":"patchset","detail":"Patchset `RT-1/P-01` is published for Task `RT-1`.","label":"Patchset","status":"done"},{"code":"attestation","command":"ait workflow ready RT-1 --apply","detail":"No attestation is recorded yet for patchset `RT-1/P-01`. Workflow ready apply owns Patchset CI completion and then records the compact Attestation gate statement.","label":"Attestation","status":"pending"}],"suggested_commands":["ait snapshot create RT-1 --message \"reviewable snapshot\"","ait patchset publish RT-1 --summary \"review summary\"","ait attest put RT-1/P-01 --tests pass"],"target_line":"main","task":{"status":"active","task_id":"RT-1"},"workflow_phase":"ready"}},{"name":"project_workflow_landed_read_model","context":{"task_review":true},"args":[{"task":{"task_id":"RT-1","status":"active"},"change":{},"patchset":{"patchset_id":"RT-1/P-01","revision_snapshot_id":"SNP-REV"},"target_line":"main","base_line_name":"main"}],"result":{"attestation":null,"base_line":{"head_snapshot_id":null,"line_name":"main"},"change":null,"freshness":{"base_is_fresh":true,"patchset_base_snapshot_id":null,"patchset_revision_snapshot_id":"SNP-REV","preflight_state":"not_required","rebase_state":"idle","recovery_required":false,"remote_base_snapshot_id":null,"worktree_needs_retarget":false},"landing_summary":null,"next_action":{"code":"workflow_ready","command":"ait workflow ready RT-1 --apply","detail":"The workspace still has unsaved changes.","summary":"Run workflow ready before review or finish."},"patchset":{"patchset_id":"RT-1/P-01","revision_snapshot_id":"SNP-REV"},"patchset_source":null,"policy":{"checks":[],"decision":"pass","status_source":"landed_fast_path"},"review":{"approvals":1,"blocking":0,"current_patchset_id":"RT-1/P-01","reviews":[],"task_approvals":1,"team_approvals":0},"steps":[{"code":"snapshot","detail":"No additional authoring snapshot is required because the change is already applied.","label":"Snapshot","status":"done"},{"code":"patchset","detail":"Patchset `RT-1/P-01` is already part of the accepted history for this change.","label":"Patchset","status":"done"},{"code":"attestation","detail":"Finish already succeeded, so attestation gating has already cleared.","label":"Attestation","status":"done"},{"code":"review","detail":"Finish already succeeded, so review requirements were already satisfied.","label":"Review","status":"done"},{"code":"policy","detail":"Finish already succeeded, so policy has already cleared.","label":"Policy","status":"done"},{"code":"land","detail":"Task `RT-1` already has accepted work applied to `main`.","label":"Finish","status":"done"}],"suggested_commands":["ait workflow ready RT-1 --apply"],"task":{"status":"active","task_id":"RT-1"},"workspace":{"changed_count":0,"clean":false,"current_line":"unknown","head_snapshot_id":null,"workspace_matches_patchset":true,"workspace_status":"dirty"}}},{"name":"project_workflow_land_full_read_model","context":{"task_review":true},"args":[{"task":{"task_id":"RT-1","status":"active"},"change":{},"patchset":{"patchset_id":"RT-1/P-01","revision_snapshot_id":"SNP-REV"},"target_line":"main","base_line_name":"main"},false],"result":{"attestation":null,"base_line":{"head_snapshot_id":null,"line_name":"main"},"change":null,"freshness":{"base_is_fresh":false,"patchset_base_snapshot_id":null,"patchset_revision_snapshot_id":null,"preflight_state":"stale","rebase_state":"idle","recovery_required":true,"remote_base_snapshot_id":null,"worktree_needs_retarget":false},"landing_summary":null,"next_action":{"code":"workflow_ready","command":"ait workflow ready RT-1 --apply","detail":"The workspace still has unsaved changes.","summary":"Run workflow ready before review or finish."},"patchset":{"patchset_id":"RT-1/P-01","revision_snapshot_id":"SNP-REV"},"patchset_ci_status":null,"patchset_refresh":null,"patchset_source":null,"policy":null,"review":{"approvals":0,"blocking":0,"task_approvals":0,"team_approvals":0},"steps":[{"code":"snapshot","command":"ait snapshot create RT-1 --message \"reviewable snapshot\"","detail":"Workspace changes are still dirty, so publishable finish state needs a fresh snapshot first.","label":"Snapshot","status":"pending"},{"code":"patchset","command":"ait patchset publish RT-1 --summary \"review summary\"","detail":"The base line `main` moved to `` after patchset `RT-1/P-01` was published.","label":"Patchset","status":"stale"},{"code":"attestation","command":"ait attest put RT-1/P-01 --tests pass","detail":"No attestation is recorded yet for patchset `RT-1/P-01`. Run workflow ready so it completes Patchset CI and records the compact Attestation gate statement.","label":"Attestation","status":"pending"},{"code":"review","command":"ait review task approve RT-1 --patchset RT-1/P-01 --message \"<functional validation>\"","detail":"Code review: not required; Task review: pending; Team review: unavailable while Agent Teams is disabled.","label":"Review","status":"pending"},{"code":"policy","detail":"Policy is folded into `task finish` for Patchset `RT-1/P-01`; finish preflight will run the authoritative evaluation.","label":"Policy","status":"advisory"},{"code":"land","command":"ait task finish RT-1","detail":"Finish waits for review to clear first.","label":"Finish","status":"waiting"}],"suggested_commands":["ait workflow ready RT-1 --apply"],"task":{"status":"active","task_id":"RT-1"},"task_review":{"value":true},"workspace":{"changed_count":0,"clean":false,"current_line":"unknown","head_snapshot_id":null,"workspace_matches_patchset":null,"workspace_status":"dirty"},"worktree_retarget":null}},{"name":"project_workflow_ready_read_model","context":{"task_review":true},"args":[{"task":{"task_id":"RT-1","status":"active"},"change":{},"patchset":{"patchset_id":"RT-1/P-01","revision_snapshot_id":"SNP-REV"},"target_line":"main","base_line_name":"main"},false,false,false],"result":{"base_line_name":"main","change":null,"next_action":{"code":"snapshot_create","command":"ait snapshot create RT-1 --message \"reviewable snapshot\"","detail":"The workspace still has unsaved changes.","summary":"Capture a fresh snapshot before publishing or refreshing the patchset."},"patchset":{"patchset_id":"RT-1/P-01","revision_snapshot_id":"SNP-REV"},"steps":[{"code":"snapshot","command":"ait snapshot create RT-1 --message \"reviewable snapshot\"","detail":"Workspace changes are still dirty, so ready state needs a fresh snapshot first.","label":"Snapshot","status":"pending"},{"code":"patchset","detail":"Patchset `RT-1/P-01` is published for Task `RT-1`.","label":"Patchset","status":"done"},{"code":"attestation","command":"ait workflow ready RT-1 --apply","detail":"No attestation is recorded yet for patchset `RT-1/P-01`. Workflow ready apply owns Patchset CI completion and then records the compact Attestation gate statement.","label":"Attestation","status":"pending"}],"suggested_commands":["ait snapshot create RT-1 --message \"reviewable snapshot\"","ait patchset publish RT-1 --summary \"review summary\"","ait attest put RT-1/P-01 --tests pass"],"target_line":"main","task":{"status":"active","task_id":"RT-1"},"workflow_phase":"ready"}}];
  for(const fixture of cases) {
    const name=fixture.name.replace(/_([a-z])/g,(_,c)=>c.toUpperCase());
    const context=Object.fromEntries(Object.entries(fixture.context).map(([key,value])=>[key.replace(/_([a-z])/g,(_,c)=>c.toUpperCase()),value]));
    assert.deepEqual(binding[name](context,...fixture.args),fixture.result,fixture.name);
  }
  assert.throws(()=>binding.projectWorkflowReadyReadModel({},[],false,false,false),/object/);
});


test('actual addon preserves workflow facts, tracking projections and native summaries', () => {
  const cases=[{"name":"build_linked_task_lookup_payload","args":[null,null],"kwargs":{},"result":{"linked_task_count":0,"task_links_by_item":[],"tasks_by_plan":[]}},{"name":"build_linked_change_lookup_payload","args":[null],"kwargs":{},"result":{"change_links_by_task":[],"linked_change_count":0}},{"name":"build_task_tracking_title_payload","args":[{"task_id":"LT-0001","title":"SDK fixture","intent":"Exercise retained data helpers","status":"active"}],"kwargs":{},"result":{"title":"LT-0001: SDK fixture"}},{"name":"build_task_tracking_metadata_payload","args":[{"task_id":"LT-0001","title":"SDK fixture","intent":"Exercise retained data helpers","status":"active"},"ai_only_experimental","required"],"kwargs":{},"result":{"author_mode":"ai_only_experimental","objective":"Exercise retained data helpers","task_id":"LT-0001","tracking_policy":"required"}},{"name":"build_task_audit_verdict_payload","args":[{"task_id":"LT-0001","title":"SDK fixture","intent":"Exercise retained data helpers","status":"active"},[],"main"],"kwargs":{},"result":{"recommended_action":{"change_id":null,"code":"create_change","detail":"This task has no linked local changes yet.","label":"Create a change"},"summary":{"ambiguous_line_change_count":0,"change_count":0,"effective_on_target_change_count":0,"effectively_complete_on_target":false,"landed_change_count":0,"line_evidence_change_count":0,"missing_remote_change_count":0,"open_change_count":0,"open_on_target_change_count":0,"patchset_count":0,"ready_to_complete":false,"stale_workflow_change_count":0,"stale_workflow_records":false,"verdict":"no_changes"},"workflow":{"reason":"No linked local changes exist yet.","state":"planning"}}},{"name":"workflow_landed_facts","args":[{}],"kwargs":{},"result":{"change":{},"current_line_name":"unknown","ignore_workspace_authoring":false,"landed":false,"landing_summary":null,"patchset":null,"patchset_label":"","patchset_revision_snapshot_id":null,"patchset_source":null,"revision_snapshot_id":null,"target_line":"main","task":{},"workspace":{}}},{"name":"workflow_land_full_facts","args":[{}],"kwargs":{},"result":{"attestation":null,"base_is_fresh":false,"base_line_name":"main","change":{},"code_review_summary_count":0,"current_line_name":"unknown","ignore_workspace_authoring":false,"landing_blocker_class":"","landing_result":{},"landing_status":"","landing_submission_id":null,"landing_summary":null,"patchset":null,"patchset_base_snapshot_id":null,"patchset_ci_status":null,"patchset_is_authoritative":false,"patchset_refresh":null,"patchset_revision_snapshot_id":null,"patchset_source":null,"policy":null,"policy_decision":"pending","remote_base_snapshot_id":null,"requires_code_review_summary":false,"resolved_change_id":"","review_approvals":0,"review_blocking":0,"review_summary":{},"revision_snapshot_id":null,"stale_policy_blocker_cleared":false,"target_line":"main","task":{},"task_review_approvals":0,"team_review_approvals":0,"tests_state":"","workspace":{},"workspace_matches_patchset":null,"worktree_retarget":null}},{"name":"workflow_ready_facts","args":[{}],"kwargs":{},"result":{"attestation":null,"base_line":{},"base_line_name":"main","change":{},"code_review_summary_count":0,"external_readiness":null,"freshness":{},"patchset":null,"patchset_ci_status":null,"patchset_refresh":null,"payload_seed":{},"policy":null,"requires_code_review_summary":false,"review_blocking":0,"task":{},"task_review_approvals":0,"task_review_enabled":false,"tests_state":"","workspace":{},"worktree_retarget":null}},{"name":"workflow_land_phase_facts","args":[{},{}],"kwargs":{},"result":{"change":{},"change_id":"","full_steps":{},"payload_seed":{},"ready_command":"ait workflow ready --apply","ready_done":false,"ready_next_action":{},"state_next_action_command":"","task":{}}},{"name":"project_workflow_land_phase_read_model","args":[{},{}],"kwargs":{},"result":{"next_action":{"code":"workflow_ready","command":"ait workflow ready --apply","detail":"The change still needs a ready patchset and attestation before finish can continue.","summary":"Run workflow ready before review or finish."},"steps":[{"code":"ready","command":"ait workflow ready --apply","detail":"Run workflow ready before review or finish.","label":"Ready","status":"pending"},{"code":"review","detail":"Review starts after `workflow ready` prepares a selected patchset and attestation.","label":"Review","status":"waiting"},{"code":"policy","detail":"Finish preflight will evaluate policy after review clears on a ready patchset.","label":"Policy","status":"waiting"},{"code":"land","detail":"Finish starts after `workflow ready` and review are complete.","label":"Finish","status":"waiting"},{"code":"task","detail":"Task completion follows a successful finish.","label":"Task","status":"waiting"}],"suggested_commands":["ait workflow ready --apply"],"workflow_phase":"land"}},{"name":"workflow_remote_mutation_receipt","args":["record_review","workflow ready","direct_response",null,{"review_id":"RV-1"}],"kwargs":{},"result":{"action":"record_review","delivery":"direct_response","review_id":"RV-1","source_action":"workflow ready"}},{"name":"workflow_remote_action_mutation_receipts","args":["record_review",{"review_id":"RV-1","policy_refresh":{"decision":"pass"}}],"kwargs":{},"result":[{"action":"record_review","delivery":"direct_response","review_id":"RV-1","source_action":"record_review"},{"action":"evaluate_policy","decision":"pass","delivery":"direct_response","source_action":"record_review"}]},{"name":"workflow_apply_phase_payload","args":["publish","published"],"kwargs":{"detail":"Fixture detail"},"result":{"code":"published","detail":"Fixture detail","phase":"publish"}},{"name":"workflow_apply_phase_summary","args":[{"phase":"publish","code":"published","detail":"Fixture detail"}],"kwargs":{},"result":"Fixture detail"},{"name":"workflow_mutation_receipt_summary","args":[{"action":"record_review","source_action":"workflow ready","delivery":"direct_response","review_id":"RV-1"}],"kwargs":{},"result":"`record_review` accepted via direct response (review_id=RV-1)"},{"name":"workflow_applied_action_summary","args":[{"code":"record_review","action":"record_review","detail":"Fixture detail"}],"kwargs":{},"result":"recorded review `unknown`"}];
  for(const fixture of cases) {
    const name=fixture.name.replace(/_([a-z])/g,(_,c)=>c.toUpperCase());
    let args=fixture.args;
    if(fixture.name==='workflow_apply_phase_payload') args=[...args,fixture.kwargs];
    if(fixture.name==='workflow_remote_mutation_receipt') {
      const [action,sourceAction,delivery,responseRecovery,result]=args;
      args=[{action,sourceAction,delivery,responseRecovery:responseRecovery ?? undefined,result}];
    }
    if(fixture.error) {
      assert.throws(()=>binding[name](...args),error=>error.message===fixture.error,fixture.name);
    } else assert.deepEqual(binding[name](...args) ?? null,fixture.result,fixture.name);
  }
  assert.throws(()=>binding.workflowReadyFacts([]),/object/);
  assert.throws(()=>binding.buildLinkedTaskLookupPayload({},null),/list|array/);
  assert.throws(()=>binding.workflowApplyPhasePayload('','ready'),/non-empty/);
  assert.throws(()=>binding.workflowRemoteActionMutationReceipts('record_review',[]),/result must be an object/);
});


test('actual addon preserves retained Task facts, file helpers and scoped apply admission', () => {
  const cases=[{"name":"task_workflow_timestamp_facts","args":["2026-10-26T00:00:00Z"],"kwargs":{},"result":{"epoch_ms":1792972800000,"source":"injected","timestamp":"2026-10-26T00:00:00Z"}},{"name":"task_workflow_sequence_identity_facts","args":["T",7],"kwargs":{"namespace_prefix":"L","width":4},"result":{"family":"T","generated_id":"LT-0007","namespace_prefix":"L","number":7,"width":4}},{"name":"task_workflow_workflow_id_facts","args":["T"],"kwargs":{"namespace_prefix":"L","timestamp_ms":42,"randomness_hex":"0123456789abcdef0123"},"result":{"family":"T","generated_id":"LT-000000001A04HMASW9NF6YY093","namespace_prefix":"L","randomness_hex":"0123456789abcdef0123","timestamp_ms":42}}];
  assert.deepEqual(binding.taskWorkflowTimestampFacts(...cases[0].args),cases[0].result);
  assert.deepEqual(binding.taskWorkflowSequenceIdentityFacts('T',7,{namespacePrefix:'L',width:4}),cases[1].result);
  assert.deepEqual(binding.taskWorkflowWorkflowIdFacts('T',{namespacePrefix:'L',timestampMs:42,randomnessHex:'0123456789abcdef0123'}),cases[2].result);
  const overrides={plan_core_backend:'rust'};
  assert.deepEqual(binding.taskWorkflowRuntimeSelectionFacts(overrides),binding.planConfigRuntimeBuildSelectionFacts(binding.encodeJson({overrides})));
  assert.throws(()=>binding.taskWorkflowTimestampFacts('invalid'),/timestamp|Timestamp/);
  assert.throws(()=>binding.taskWorkflowSequenceIdentityFacts('T',9223372036854775808n),/must fit in i64/);
  assert.throws(()=>binding.taskWorkflowWorkflowIdFacts('T',{timestampMs:42}),/randomness_hex/);
  assert.throws(()=>binding.taskWorkflowRuntimeSelectionFacts([]),/object/);
  assert.equal(binding.taskRemoteChangeLineagePayload('main',{head_snapshot_id:'SNP-BASE'}).fork_snapshot_id,'SNP-BASE');
  const fixture=fs.realpathSync(fs.mkdtempSync(path.join(os.tmpdir(),'ait-task-sdk-'))),root=path.join(fixture,'repo');
  fs.mkdirSync(root);
  function writableDirectories(folder) {
    fs.chmodSync(folder,0o700);
    for(const entry of fs.readdirSync(folder,{withFileTypes:true})) {
      if(entry.isDirectory() && !entry.isSymbolicLink()) writableDirectories(path.join(folder,entry.name));
    }
  }
  try {
    binding.taskWorkflowInit(root);
    binding.configureRepository(root,{agent_teams:'disabled',user_name:'binding fixture'});
    const configPath=path.join(root,'.ait','config.json'),config=JSON.parse(fs.readFileSync(configPath));
    config.task_worktree={ephemeral_root:path.join(fixture,'worktrees'),alias_root:path.join(fixture,'aliases')};
    fs.writeFileSync(configPath,JSON.stringify(config));
    const textPath=path.join(root,'text.txt'),bytesPath=path.join(root,'bytes.bin'),jsonPath=path.join(root,'value.json');
    fs.writeFileSync(textPath,'text\r\n');fs.writeFileSync(bytesPath,Buffer.from([0,255,128]));
    fs.writeFileSync(jsonPath,'{"n":18446744073709551615}');
    assert.equal(binding.taskWorkflowReadUtf8TextFile(textPath),'text\n');
    assert.deepEqual(binding.taskWorkflowReadBinaryFile(bytesPath),Buffer.from([0,255,128]));
    assert.equal(binding.taskWorkflowReadJsonFile(jsonPath).n,18446744073709551615n);
    assert.equal(binding.taskWorkflowResolveRepoArtifactPath(root,'text.txt').artifact_path,'text.txt');
    assert.throws(()=>binding.taskWorkflowResolveRepoArtifactPath(root,'../outside'),/Invalid|outside/);
    assert.throws(()=>binding.taskWorkflowReadUtf8TextFile(bytesPath),/Io\(|Invalid\(/);
    const skipped=binding.taskWorkflowEnsureMainSeedMirror(root,{lineName:'other'});
    assert.equal(skipped.reason,'target_not_default_line');
    const seed=binding.taskWorkflowResolveMainSeedMirrorLocation(root,'fixture-seed');
    assert.ok(seed===null || seed.target_path.startsWith(fixture+path.sep));
    const task=binding.taskQuick(root,{intent:'Exercise retained Task SDK',local:true,editRoot:path.join(fixture,'work')});
    const location=binding.taskWorkflowResolveTaskWorktreeLocation(root,task.worktree.name);
    assert.ok(location.target_path.startsWith(fixture+path.sep));
    fs.writeFileSync(path.join(task.worktree.path,'sdk.txt'),'Retained SDK content\n');
    const snapshot=binding.snapshotCapture(task.worktree.path,task.task_id,{message:'Retained SDK fixture'});
    assert.equal(binding.taskWorkflowEnsureStatusManifest(root,snapshot.snapshot_id).source,'snapshot_binary_metadata');
    assert.equal(binding.taskWorkflowTaskAudit(root,task.task_id,{local:true}).task.task_id,task.task_id);
    assert.throws(()=>binding.taskWorkflowTaskLand(root,task.task_id,{local:true}),/task-bound worktree/);
    const applied=binding.taskWorkflowTaskLand(task.worktree.path,task.task_id,{local:true});
    assert.equal(applied.task_id,task.task_id);
    assert.equal(binding.taskWorkflowTaskShow(root,task.task_id,{local:true}).status,'completed');
    assert.throws(()=>binding.taskWorkflowTaskAbandon(root,'LT-9999',{local:true}),/Unknown.*[Tt]ask|not found/);
    for(const call of [
      ()=>binding.taskWorkflowQueueSummary(root,'absent'),
      ()=>binding.taskWorkflowLandSubmit(root,'RT-1',{targetLine:'main',mode:'direct',remoteName:'absent'}),
      ()=>binding.taskWorkflowLandShow(root,'receipt',{remoteName:'absent'}),
      ()=>binding.taskWorkflowLandRetry(root,'receipt',{remoteName:'absent'}),
      ()=>binding.workflowReadyPayloadDirect(root,task.task_id,'absent'),
      ()=>binding.workflowLandPayloadDirect(root,task.task_id,'absent'),
      ()=>binding.workflowReadyApplyDirect(root,task.task_id,{remoteName:'absent'}),
      ()=>binding.workflowLandApplyDirect(root,task.task_id,{remoteName:'absent'}),
    ]) assert.throws(call,/absent/);
  } finally {
    writableDirectories(fixture);
    fs.rmSync(fixture,{recursive:true,force:true});
  }
});


test('actual addon preserves runtime/manifest/transport data helpers and usage extraction', () => {
  const cases=[{"name":"runtime_binding_state_normalize_document","args":[{}],"kwargs":{},"result":{"ir_version":"ait.runtime_binding_state.v2","state":{"last_update_id":0,"surface_bindings":{},"telegram_bootstrap_auth":{},"version":4}}},{"name":"worker_manifest_normalize_document","args":[{}],"kwargs":{},"result":{"config":{"version":1,"workers":{}},"ir_version":"ait.worker_manifest.v1","issues":["Worker config at <memory> is missing or has invalid `workers` map."]}},{"name":"worker_manifest_select_telegram_worker","args":[],"kwargs":{"config":{"version":1,"workers":{}}},"result":null},{"name":"transport_envelope_build_binding_metadata","args":[],"kwargs":{"transport":"fixture","surface_id":"surface","conversation_key":"conversation","reply_target":{"id":42}},"result":{"conversation_key":"conversation","surface_id":"surface","transport":"fixture","transport_reply_target":{"id":42}}},{"name":"transport_envelope_build_event_envelope","args":[],"kwargs":{"transport":"fixture","actor_identity":"actor","channel_id":"channel","text":"Hello","occurred_at":"2026-10-26T00:00:00Z","event_id":"E-1","dedupe_key":"stable","message_id":42},"result":{"actor":{"actor_identity":"actor"},"channel":{"channel_id":"channel"},"dedupe_key":"stable","event_id":"E-1","event_kind":"message","message":{"logical_turn_message_count":1,"message_id":42,"message_ids":[42],"text":"Hello"},"occurred_at":"2026-10-26T00:00:00Z","schema_version":1,"transport":"fixture"}},{"name":"transport_envelope_build_reply_envelope","args":[],"kwargs":{"transport":"fixture","channel_id":"channel","text":"Reply","reply_to_event_id":"E-1","reply_to_message_id":42},"result":{"delivery_kind":"chat_reply","message":{"text":"Reply"},"reply_to":{"event_id":"E-1","logical_turn_message_count":1,"message_id":42,"message_ids":[42]},"schema_version":1,"target":{"channel_id":"channel"},"transport":"fixture"}}];
  for(const fixture of cases) {
    const name=fixture.name.replace(/_([a-z])/g,(_,c)=>c.toUpperCase());
    const kwargs=Object.fromEntries(Object.entries(fixture.kwargs).map(([key,value])=>[key.replace(/_([a-z])/g,(_,c)=>c.toUpperCase()),value]));
    const args=fixture.name==='worker_manifest_select_telegram_worker' ? [kwargs.config] :
      fixture.name.startsWith('transport_envelope_build') ? [kwargs] : fixture.args;
    assert.deepEqual(binding[name](...args),fixture.result,fixture.name);
  }
  const expected={"event":{"channel":{"channel_id":"channel"},"event_id":"E-1","event_kind":"message","message":{"logical_turn_message_count":1,"message_id":42,"message_ids":[42]},"transport":"fixture"},"reply":{"reply_to":{"event_id":"E-1","message_id":42,"message_ids":[42]},"target":{"channel_id":"channel"},"transport":"fixture"},"state":{"last_update_id":0,"surface_bindings":{},"telegram_bootstrap_auth":{},"version":4},"manifest":{"version":1,"workers":{}}};
  assert.deepEqual(binding.runtimeBindingStateDefaultPayload(),expected.state);
  assert.deepEqual(binding.workerManifestDefaultConfig(),expected.manifest);
  assert.deepEqual(binding.transportEnvelopeCompactEventEnvelope(cases[4].result),expected.event);
  assert.deepEqual(binding.transportEnvelopeCompactReplyEnvelope(cases[5].result),expected.reply);
  assert.deepEqual(binding.languageBindingInfo(),JSON.parse(binding.bindingInfoJson()));
  for(const prefix of ['transportEnvelope','runtimeBindingState','workerManifest']) {
    assert.equal(binding[prefix+'Schema']().$id,'https://ait.dev/schema/'+binding[prefix+'IrVersion']()+'.schema.json');
  }
  const event=binding.transportEnvelopeBuildEventEnvelope({transport:'fixture',actorIdentity:'actor',channelId:'channel',text:'Message',messageId:9223372036854775807n});
  assert.equal(event.message.message_id,9223372036854775807n);
  assert.deepEqual(binding.projectRepoRetireRuntimeBlockers({}),{});
  assert.throws(()=>binding.projectRepoRetireRuntimeBlockers([]),/must be an object/);
  for(const retired of ['aitAgentWorkerCapabilities','aitAgentManagement','aitAgentWorkerTransaction','aitAgentWebRuntimeExecute','aitAgentEnvFileLoad','aitAgentTelegramWorkflowNotificationFormat','aitAgentTelegramTurnInputPlan','aitAgentTelegramMessageDeliveryExecute','aitAgentTelegramWorkflowQueryPlan']) {
    assert.equal(binding[retired],undefined);
  }
  const root=fs.realpathSync(fs.mkdtempSync(path.join(os.tmpdir(),'ait-usage-sdk-')));
  try {
    const file=path.join(root,'usage.jsonl');
    fs.writeFileSync(file,JSON.stringify({type:'turn.completed',usage:{input_tokens:100,output_tokens:20,cached_input_tokens:30,reasoning_output_tokens:5}})+'\n');
    const usage=binding.extractCodexUsageJsonl(file,'total');
    assert.equal(usage.token_event_count,1);
    assert.equal(usage.manifest_usage.prompt_tokens,100);
    assert.equal(usage.manifest_usage.completion_tokens,20);
    assert.equal(usage.manifest_usage.total_tokens,120);
    const bundle=binding.extractCodexUsageBundleJsonl([file,file],{usageRoles:['Author','reviewer']});
    assert.equal(bundle.usage_file_count,2);
    assert.equal(bundle.manifest_usage.total_tokens,240);
    assert.throws(()=>binding.extractCodexUsageBundleJsonl([file],{usageRoles:[]}),/must match/);
    assert.throws(()=>binding.extractCodexUsageBundleJsonl([]),/At least one/);
    assert.throws(()=>binding.extractCodexUsageJsonl(file,'unknown'),/Unsupported Codex usage scope/);
    fs.writeFileSync(file,'{');
    assert.throws(()=>binding.extractCodexUsageJsonl(file,'total'),/not valid JSONL/);
    assert.throws(()=>binding.extractCodexUsageJsonl(path.join(root,'absent'),'total'),/not found/);
  } finally {fs.rmSync(root,{recursive:true,force:true});}
});


test('actual addon preserves task remote pack bytes, codecs and client lifecycle', async () => {
  const worker=new Worker(`
    const {parentPort}=require('node:worker_threads');const http=require('node:http');const packs=new Map();
    const server=http.createServer((req,res)=>{
      const chunks=[];req.on('data',chunk=>chunks.push(chunk));req.on('end',()=>{
        res.setHeader('content-type','application/json');
        const pathname=new URL(req.url,'http://fixture').pathname;
        if(pathname.includes('/object-packs/')||pathname.includes('/tree-packs/')) {
          if(req.method==='PUT') {packs.set(pathname,Buffer.concat(chunks));res.end(JSON.stringify({pack_id:pathname.split('/').pop(),stored:true,pack_bytes:Buffer.concat(chunks).length}));}
          else {res.setHeader('content-type','application/octet-stream');res.end(packs.get(pathname)||Buffer.from([0,255,128,42]));}
        } else if(pathname.endsWith('/plan')) {
          res.end(JSON.stringify({present_snapshot_ids:[],missing_snapshot_ids:['SNP-BASE'],present_object_pack_ids:[],missing_object_pack_ids:[],present_tree_pack_ids:[],missing_tree_pack_ids:[]}));
        } else if(pathname.endsWith('/commit')) {
          res.end(JSON.stringify({committed_snapshot_ids:[],committed_object_pack_ids:[],committed_tree_pack_ids:[],upserted_snapshots:0}));
        } else if(pathname.includes('/import-manifests/')) {res.end('{"contract":"invalid"}');}
        else if(pathname.endsWith('/lines/main')) {res.end('{"line_name":"main","head_snapshot_id":"SNP-BASE"}');}
        else if(pathname.endsWith('/tasks/RT-1')) {res.end('{"task_id":"RT-1","status":"active","successful_land":null,"current_patchset_id":null}');}
        else if(pathname.endsWith('/tasks')) {res.end('[{"task_id":"RT-1","status":"active"}]');}
        else if(pathname.includes('/snapshots/')) {res.end('{"snapshot_id":"SNP-BASE","files":[]}');}
        else {res.end('{}');}
      });
    });server.listen(0,'127.0.0.1',()=>parentPort.postMessage(server.address().port));
  `,{eval:true});
  try {
    const [port]=await once(worker,'message');
    const remote=new binding.HttpTaskRemote(`http://127.0.0.1:${port}`,{repositoryIndex:7});
    assert.equal(remote.changeLineagePayload('main',{head_snapshot_id:'SNP-BASE'}).fork_snapshot_id,'SNP-BASE');
    assert.equal(remote.getLine('fixture','main').head_snapshot_id,'SNP-BASE');
    assert.equal(remote.getTask('RT-1').task_id,'RT-1');
    assert.equal(remote.listTasks('fixture')[0].task_id,'RT-1');
    const plan=remote.planRemoteZstdBulk('fixture',{snapshot_ids:['SNP-BASE'],object_packs:[],tree_packs:[]});
    assert.deepEqual(plan.missing_snapshot_ids,['SNP-BASE']);
    const bytes=Buffer.from([0,255,128,42]);
    assert.throws(()=>remote.putRemoteZstdObjectPack('fixture','PCK-BYTES',bytes),/Actor registration/);
    assert.deepEqual(remote.getRemoteZstdObjectPack('fixture','PCK-BYTES'),bytes);
    assert.throws(()=>remote.putRemoteZstdTreePack('fixture','TPK-BYTES',bytes),/Actor registration/);
    assert.deepEqual(remote.getRemoteZstdTreePack('fixture','TPK-BYTES'),bytes);
    assert.throws(()=>remote.commitRemoteZstdBulk('fixture',{object_packs:[],tree_packs:[],blob_locators:[],tree_locators:[],snapshots:[]}),/Actor registration/);
    assert.equal(remote.getRemoteSnapshot('fixture','SNP-BASE',{includeContent:true,path:'a.txt'}).snapshot_id,'SNP-BASE');
    assert.throws(()=>remote.getRemoteZstdImportManifest('fixture','SNP-BASE'),/Missing required field `repo_name`/);
    assert.throws(()=>remote.planRemoteZstdBulk('fixture',{snapshot_ids:'invalid'}),/array|list/);
    assert.ok(remote.inspectClient().request_count>=8);
    assert.equal(remote.closeClient().closed,true);
    assert.throws(()=>remote.getTask('RT-1'),/already closed/);
  } finally {await worker.terminate();}
});

test('actual addon executes retained Plan HTTP transport and native mutation guards', async () => {
  const worker = new Worker(`
    const {parentPort}=require('node:worker_threads');
    const http=require('node:http'); const calls=[];
    const server=http.createServer((req,res)=>{
      const chunks=[];req.on('data',chunk=>chunks.push(chunk));req.on('end',()=>{
        calls.push({url:req.url,method:req.method,body:Buffer.concat(chunks).toString(),header:req.headers['x-fixture']});
        res.setHeader('content-type','application/json');
        const pathname=new URL(req.url,'http://fixture').pathname;
        if(pathname.endsWith('/error')) {res.statusCode=409;res.end('{"error":"fixture rejection"}');return;}
        if(pathname.endsWith('/badshape')) {res.end('[]');return;}
        if(req.method==='GET' && pathname.endsWith('/events')) {res.end('[{"sequence":9223372036854775807}]');return;}
        if(req.method==='GET' && ['/sprints','/revisions','/planning-sessions'].some(suffix=>pathname.endsWith(suffix))) {
          res.end('[{"plan_id":"P-1"}]');return;
        }
        if(pathname.endsWith('/sprint-plan-ids/by-contains')) {res.end('["P-1"]');return;}
        res.end(JSON.stringify({ok:true,path:pathname,method:req.method}));
      });
    });
    server.listen(0,'127.0.0.1',()=>parentPort.postMessage(server.address().port));
    parentPort.on('message',message=>{if(message==='calls')parentPort.postMessage(calls);});
  `,{eval:true});
  try {
    const [port]=await once(worker,'message'), base=`http://127.0.0.1:${port}`;
    const options={repositoryIndex:7n,headersJson:'{"X-Fixture":"native"}',timeoutMs:0n,retryAttempts:0,retryBackoffMs:0n,poolMaxIdlePerHost:1};
    assert.equal(binding.listPlans(base,'fixture','docs/a b.md',options)[0].plan_id,'P-1');
    assert.equal(binding.getPlan(base,'P/1',options).ok,true);
    assert.equal(binding.listPlanRevisions(base,'P-1',options).length,1);
    assert.equal(binding.getPlanRevision(base,'P-1','R/1',options).ok,true);
    assert.equal(binding.resolveTaskPlanLinkage(base,'fixture',{planId:'P-1',originPlanRevisionId:'R-1',planItemRef:'item'},options).ok,true);
    assert.deepEqual(binding.listPlanIdsMatchingContains(base,'fixture',['one',' two '],options),['P-1']);
    const create={repoName:'fixture',title:'Title',artifactPath:'docs/a.md',artifactHeading:'Heading',items:[]};
    const revise={planId:'P-1',artifactPath:'docs/a.md',artifactHeading:'Heading',items:[]};
    for(const operation of [
      ()=>binding.createPlan(base,create,options),()=>binding.revisePlan(base,revise,options),
      ()=>binding.updatePlanStatus(base,'P-1','draft',options),
      ()=>binding.putPlanRevisionArtifacts(base,'P-1','R-1',[],options),
    ]) assert.throws(operation,/Actor registration/);
    assert.equal(binding.createPlanningSession(base,'P-1',undefined,options).ok,true);
    assert.equal(binding.listPlanningSessions(base,'P-1','active',options).length,1);
    assert.equal(binding.getPlanningSession(base,'PS-1',options).ok,true);
    assert.equal(binding.appendPlanningSessionEvent(base,'PS-1','note',{value:18446744073709551615n},options).ok,true);
    assert.equal(binding.listPlanningSessionEvents(base,'PS-1',{afterSequence:9223372036854775807n,limit:5},options)[0].sequence,9223372036854775807n);
    assert.equal(binding.joinPlanningSession(base,'PS-1',undefined,options).ok,true);
    assert.equal(binding.promotePlanningSession(base,'PS-1',{artifactPath:'docs/a.md',artifactSelector:'section',artifactHeading:'Heading',items:[]},options).ok,true);
    assert.equal(binding.closePlanningSession(base,'PS-1',undefined,options).ok,true);
    const manager=new binding.PlanHttpClientManager(base,options);
    assert.equal(manager.inspect().request_count,0);
    assert.equal(manager.listPlans('fixture')[0].plan_id,'P-1');
    assert.equal(manager.getPlan('P-1').ok,true);
    assert.equal(manager.listPlanRevisions('P-1').length,1);
    assert.equal(manager.getPlanRevision('P-1','R-1').ok,true);
    for(const operation of [()=>manager.createPlan(create),()=>manager.revisePlan(revise),
      ()=>manager.updatePlanStatus('P-1','draft'),()=>manager.putPlanRevisionArtifacts('P-1','R-1',[])]) {
      assert.throws(operation,/Actor registration/);
    }
    assert.equal(manager.createPlanningSession('P-1').ok,true);
    assert.equal(manager.listPlanningSessions('P-1').length,1);
    assert.equal(manager.getPlanningSession('PS-1').ok,true);
    assert.equal(manager.appendPlanningSessionEvent('PS-1','note').ok,true);
    assert.equal(manager.listPlanningSessionEvents('PS-1')[0].sequence,9223372036854775807n);
    assert.equal(manager.joinPlanningSession('PS-1').ok,true);
    assert.equal(manager.promotePlanningSession('PS-1',{artifactPath:'docs/a.md',artifactSelector:'section',artifactHeading:'Heading',items:[]}).ok,true);
    assert.equal(manager.closePlanningSession('PS-1').ok,true);
    assert.equal(manager.inspect().request_count,12);
    assert.equal(manager.close().closed,true);
    assert.deepEqual(manager.close(),manager.inspect());
    assert.throws(()=>manager.getPlan('P-1'),/already closed/);
    const workflowManager=new binding.TaskWorkflowHttpClientManager(base,options);
    assert.equal(workflowManager.inspect().request_count,0);
    assert.equal(workflowManager.close().closed,true);
    const timeouts=new binding.PlanRemoteTransportConfig();
    assert.equal(binding.DEFAULT_REQUEST_TIMEOUT_SECONDS,30);
    assert.equal(binding.LONG_RUNNING_REQUEST_TIMEOUT_SECONDS,1800);
    assert.equal(timeouts.requestTimeoutSeconds(false),30);
    assert.equal(timeouts.requestTimeoutSeconds(true),1800);
    assert.equal(new binding.PlanRemoteTransportConfig(2,4).requestTimeoutSeconds(true),4);
    assert.throws(()=>new binding.PlanRemoteTransportConfig(Infinity),/expected positive finite seconds/);
    assert.throws(()=>new binding.PlanRemoteTransportConfig(undefined,0),/long_running_request/);
    assert.throws(()=>binding.getPlan(base,'error',options),/409.*fixture rejection/);
    assert.throws(()=>binding.getPlan(base,'badshape',options),/expected an object payload/);
    assert.throws(()=>binding.listPlans(base,'fixture'),/repository_index is required/);
    assert.throws(()=>binding.getPlan(base,'P-1',{headersJson:'{'}),/headers_json must be valid JSON/);
    assert.throws(()=>binding.getPlan(base,'P-1',{headersJson:'[]'}),/headers_json must contain a JSON object/);
    assert.throws(()=>binding.getPlan(base,'P-1',{headersJson:'{"x":1}'}),/headers_json values must be non-empty strings/);
    assert.throws(()=>binding.getPlan(base,'P-1',{repositoryIndex:4294967296n}),/must fit in u32/);
    assert.throws(()=>binding.getPlan(base,'P-1',{timeoutMs:-1}),/must fit in u64/);
    assert.throws(()=>binding.listPlanningSessionEvents(base,'PS-1',{afterSequence:9223372036854775808n},options),/must fit in i64/);
    assert.throws(()=>binding.appendPlanningSessionEvent(base,'PS-1','note',[],options),/payload must be an object/);
    worker.postMessage('calls');const [calls]=await once(worker,'message');
    assert.ok(calls.every(call=>call.header==='native'));
    assert.ok(calls.some(call=>call.url.includes('/sprints/P%2F1')));
    assert.ok(calls.some(call=>call.url.includes('/revisions/R%2F1')));
    assert.ok(calls.some(call=>call.url.includes('after_sequence=9223372036854775807')));
    assert.ok(calls.some(call=>call.body.includes('18446744073709551615')));
    assert.ok(calls.some(call=>call.body.includes('"mode":"connected_local"')));
    assert.ok(calls.some(call=>call.body.includes('"surface":"cli"')));
    assert.ok(calls.some(call=>call.body.includes('"status":"closed"')));
  } finally {await worker.terminate();}
});

test('actual addon preserves retained Plan HTTP request-builder payloads', () => {
  const cases=[{"name":"join_path","args":["https://example.test///","///a b"],"kwargs":{},"result":"https://example.test/a b"},{"name":"build_query_string","args":[{"z":"a b","a":"中/+"}],"kwargs":{},"result":"a=%E4%B8%AD%2F%2B&z=a+b"},{"name":"build_query_string","args":[{}],"kwargs":{},"result":null},{"name":"build_list_plans_request","args":["https://example.test///"," demo "," docs/a b.md "],"kwargs":{},"result":{"method":"GET","url":"https://example.test/v1/native/repositories/ demo /sprints?artifact_path=docs%2Fa+b.md","body":null}},{"name":"build_get_plan_request","args":["https://example.test///"," P/R "],"kwargs":{},"result":{"method":"GET","url":"https://example.test/v1/native/sprints/ P/R ","body":null}},{"name":"build_list_plan_revisions_request","args":["https://example.test///","P-1"],"kwargs":{},"result":{"method":"GET","url":"https://example.test/v1/native/sprints/P-1/revisions","body":null}},{"name":"build_get_plan_revision_request","args":["https://example.test///","P-1","R/1"],"kwargs":{},"result":{"method":"GET","url":"https://example.test/v1/native/sprints/P-1/revisions/R/1","body":null}},{"name":"build_create_plan_request","args":["https://example.test///","repo","Title","docs/a.md",null,"Heading",[{"n":18446744073709551615n}]],"kwargs":{"summary":"","artifact_body":"\ntext\n"},"result":{"method":"POST","url":"https://example.test/v1/native/repositories/repo/sprints","body":{"artifact_body":"\ntext\n","artifact_heading":"Heading","artifact_path":"docs/a.md","artifact_selector":null,"items":[{"n":18446744073709551615n}],"source_kind":"manual_edit","status":"draft","summary":"","title":"Title"}}},{"name":"build_revise_plan_request","args":["https://example.test///","P-1","docs/b.md","selector","Heading",[]],"kwargs":{"title":"","expected_head_revision_id":"R-1"},"result":{"method":"POST","url":"https://example.test/v1/native/sprints/P-1/revisions","body":{"artifact_heading":"Heading","artifact_path":"docs/b.md","artifact_selector":"selector","expected_head_revision_id":"R-1","items":[],"source_kind":"manual_edit","title":""}}},{"name":"build_update_plan_status_request","args":["https://example.test///","P-1"," draft "],"kwargs":{},"result":{"method":"PATCH","url":"https://example.test/v1/native/sprints/P-1","body":{"status":" draft "}}},{"name":"build_put_plan_revision_artifacts_request","args":["https://example.test///","P-1","R-1",[{"blob_id":"B-1"}]],"kwargs":{},"result":{"method":"PUT","url":"https://example.test/v1/native/sprints/P-1/revisions/R-1/artifacts","body":{"artifacts":[{"blob_id":"B-1"}]}}}];
  for (const fixture of cases) {
    const name=fixture.name.replace(/_([a-z])/g,(_,char)=>char.toUpperCase());
    let args=fixture.args;
    if (fixture.name==='build_create_plan_request') {
      const [base,repoName,title,artifactPath,artifactSelector,artifactHeading,items]=args;
      args=[base,{repoName,title,artifactPath,artifactSelector:artifactSelector ?? undefined,artifactHeading,items,summary:fixture.kwargs.summary,artifactBody:fixture.kwargs.artifact_body}];
    } else if (fixture.name==='build_revise_plan_request') {
      const [base,planId,artifactPath,artifactSelector,artifactHeading,items]=args;
      args=[base,{planId,artifactPath,artifactSelector,artifactHeading,items,title:fixture.kwargs.title,expectedHeadRevisionId:fixture.kwargs.expected_head_revision_id}];
    }
    const result=binding[name](...args);
    assert.deepEqual(result ?? null,fixture.result,fixture.name);
  }
  assert.equal(binding.normalizeRemoteText('  native  '),'native');
  assert.equal(binding.normalizeRemoteText('  ') ?? null,null);
  assert.equal(binding.normalizeRemoteText(null) ?? null,null);
  assert.equal(binding.buildListPlansRequest('https://example.test','repo',null).body,null);
  assert.throws(()=>binding.buildQueryString({key:null}),/values must be strings/);
  assert.throws(()=>binding.buildGetPlanRequest('https://example.test',''),/missing required value: plan_id/);
  assert.throws(()=>binding.buildCreatePlanRequest('https://example.test',{
    repoName:'',title:'',artifactPath:'',artifactHeading:'',items:{},
  }),/missing required value: repo_name/);
  assert.throws(()=>binding.buildCreatePlanRequest('https://example.test',{
    repoName:'repo',title:'Title',artifactPath:'docs/a.md',artifactHeading:'Heading',items:{},
  }),/items must be a list/);
});


test('actual addon preserves storage identities and executes object/tree pack SDK operations', () => {
  const baseline = {"entries":[{"path":"a.txt","blob_id":"BLB-A","size_bytes":1,"mode":"0o644"},{"path":"nested/b.bin","blob_id":"BLB-B","size_bytes":4,"mode":"0o755"}],"trees":{"root_tree_id":"TRE-59B20845E41B99DD6AF0","tree_entry_rows":[{"entry_name":"a.txt","entry_type":"blob","mode":"0o644","size_bytes":1,"target_id":"BLB-A","tree_id":"TRE-59B20845E41B99DD6AF0"},{"entry_name":"nested","entry_type":"tree","mode":"tree","size_bytes":null,"target_id":"TRE-66DCFD925E8A6C6844CB","tree_id":"TRE-59B20845E41B99DD6AF0"},{"entry_name":"b.bin","entry_type":"blob","mode":"0o755","size_bytes":4,"target_id":"BLB-B","tree_id":"TRE-66DCFD925E8A6C6844CB"}],"tree_rows":[{"entry_count":2,"tree_id":"TRE-59B20845E41B99DD6AF0"},{"entry_count":1,"tree_id":"TRE-66DCFD925E8A6C6844CB"}]},"snapshot_inputs":{"repo_name":"fixture","line_name":"main","parent_snapshot_id":null,"message":"SDK","root_tree_id":"TRE-59B20845E41B99DD6AF0","snapshot_kind":"line"},"snapshot":{"revision_hash":"2e95af56f25d3a1dbc7c62ace62f6bf7fe5bf3f590d6097eae0904a1d85a62e3","snapshot_id":"SNP-2E95AF56F25D"}};
  const trees = binding.planPackSubstrateBuildTreeRecords(baseline.entries);
  assert.deepEqual(trees, baseline.trees);
  assert.deepEqual(binding.planPackSubstrateBuildTreeRecords([...baseline.entries].reverse()), trees);
  assert.deepEqual(binding.planPackSubstrateBuildSnapshotId({
    repoName:'fixture',lineName:'main',message:'SDK',rootTreeId:trees.root_tree_id,
  }), baseline.snapshot);
  assert.throws(() => binding.planPackSubstrateBuildTreeRecords({}), /file_entries must be a list/);
  assert.throws(() => binding.planPackSubstrateBuildTreeRecords([
    {path:'a',blob_id:'BLB-A',mode:'0o644'},{path:'a/b',blob_id:'BLB-B',mode:'0o644'},
  ]), /Path collision/);
  assert.throws(() => binding.planPackSubstrateBuildTreeRecords([
    {path:'a',blob_id:'BLB-A',mode:'0o644',size_bytes:9223372036854775808n},
  ]), /must fit in i64/);
  const base = Buffer.alloc(512, 0xff), target = Buffer.from(base);
  target.set([0,1,2,3,4,5],240);
  const delta = binding.planPackSubstrateBuildGitBinaryDelta(base,target);
  assert.ok(Buffer.isBuffer(delta));
  assert.deepEqual(binding.planPackSubstrateApplyGitBinaryDelta(base,delta),target);
  assert.deepEqual(binding.planPackSubstrateApplyPackDelta(base,delta,'git-binary-v1'),target);
  assert.throws(() => binding.planPackSubstrateApplyPackDelta(base,delta,'unknown'), /Unsupported/);
  assert.throws(() => binding.planPackSubstrateApplyGitBinaryDelta(base,Buffer.alloc(0)));
  const deltaMember = binding.planPackSubstrateBuildGitBinaryDeltaMember('blobs/BLB-B','BLB-B','BLB-A',base,target,1n);
  assert.equal(deltaMember.entry_type,'delta');
  assert.deepEqual(Buffer.from(deltaMember.logical_data), target);
  assert.throws(() => binding.planPackSubstrateBuildGitBinaryDeltaMember('b','b','a',base,target,-1),/native-size/);
  assert.throws(() => binding.planPackSubstrateBuildPackMembers([], {maxDeltaChainDepth:18446744073709551616n}), /range|u64/);
  const members = binding.planPackSubstrateBuildPackMembers([
    {entry_name:'blobs/BLB-A',blob_id:'BLB-A',data:[...base],path_hint:'data.bin'},
    {entry_name:'blobs/BLB-B',blob_id:'BLB-B',data:[...target],path_hint:'data.bin'},
  ]);
  assert.equal(members[1].entry_type,'delta');
  const root = fs.realpathSync(fs.mkdtempSync(path.join(os.tmpdir(),'ait-storage-sdk-')));
  try {
    const stamp='2026-04-12T00:00:00+00:00';
    const objectPath=path.join(root,'objects.zstpack'), treePath=path.join(root,'trees.zstpack');
    const objectWritten=binding.planPackSubstrateWritePackArchive(objectPath,'PCK-TEST',stamp,members);
    const objectIndex=binding.planPackSubstrateReadPackIndex(objectPath);
    assert.equal(objectIndex.pack_id,'PCK-TEST');
    assert.equal(binding.planPackSubstratePackHasEntry(objectPath,'blobs/BLB-B'),true);
    assert.equal(binding.planPackSubstratePackHasEntry(objectPath,'missing'),false);
    const objectSummary=binding.planPackSubstrateSummarizePackArchives(root,[
      {pack_path:'objects.zstpack',pack_format:objectWritten.pack_format},
    ]);
    assert.equal(objectSummary.indexed_pack_count,1);
    assert.equal(objectSummary.pack_indexed_blob_count,2);
    assert.equal(objectSummary.index_error_count,0);
    const treeMembers=binding.planPackSubstrateBuildTreePackMembers(trees.tree_rows,trees.tree_entry_rows);
    const treeWritten=binding.planPackSubstrateWriteTreePackArchive(treePath,'TPK-TEST',stamp,treeMembers);
    const treeIndex=binding.planPackSubstrateReadTreePackIndex(treePath);
    assert.equal(treeIndex.tree_count,2);
    const rootRows=binding.planPackSubstrateReadTreePackTree(treePath,trees.root_tree_id);
    assert.equal(rootRows.length,2);
    const ordinal=binding.planPackSubstrateReadTreePackTreeByOrdinal(treePath,0n);
    assert.deepEqual(ordinal.rows,binding.planPackSubstrateReadTreePackTree(treePath,ordinal.tree_id));
    assert.throws(() => binding.planPackSubstrateReadTreePackTreeByOrdinal(treePath,99),/missing/);
    assert.equal(typeof binding.planPackSubstrateTreePackManifestPath(treePath,'trees/example.json'),'string');
    const treeSummary=binding.planPackSubstrateSummarizeTreePackArchives(root,[
      {pack_path:'trees.zstpack',pack_format:treeWritten.pack_format},
    ]);
    assert.equal(treeSummary.indexed_tree_count,2);
    assert.equal(treeSummary.indexed_entry_count,3);
    assert.equal(treeSummary.index_error_count,0);
    assert.equal(binding.planPackSubstrateSummarizePackArchives(root,[{pack_path:'absent',pack_format:objectWritten.pack_format}]).index_error_count,1);
    const validation={packedBlobCount:2,packedFullBlobCount:1,packedDeltaBlobCount:1,packCount:1,packIndexErrorCount:0,storageSavingsRatio:0.5};
    assert.equal(binding.planPackSubstrateBuildStorageValidationSummary(validation).needs_attention,false);
    assert.equal(binding.planPackSubstrateBuildStorageValidationSummary({...validation,signalsSummary:{drift_count:1}}).needs_attention,true);
    assert.equal(binding.planPackSubstrateBuildStorageValidationSummary({...validation,treePackIndexErrorCount:1}).needs_attention,true);
  } finally { fs.rmSync(root,{recursive:true,force:true}); }
});


test('actual addon exposes direct functions and retains chat retirement', () => {
  assert.equal(binding.runCli, undefined);
  assert.equal(binding.taskWorkflowRepoCommand, undefined);
  assert.equal(JSON.parse(binding.bindingInfoJson()).contract, 'ait.language.binding.v1');
  assert.throws(() => binding.agentWorkerCapabilitiesJson(), {
    message: 'Chat integrations are retired in AIT 1.2.0',
    code: 'GenericFailure',
  });
  assert.equal(typeof binding.planTimeIdentityBuildTimestamp('{}').timestamp, 'string');
  assert.throws(() => binding.planTimeIdentityBuildTimestamp('{'), error =>
    error.code === 'GenericFailure' && Boolean(error.message));
});

test('actual addon emits native Task progress when the language CLI requests it', () => {
  // This child is a test program loading the addon directly, not an AIT CLI transport.
  const result = spawnSync(process.execPath, ['--input-type=module', '-e', `
    import fs from 'node:fs'; import os from 'node:os'; import path from 'node:path';
    const addon = {exports:{}}; process.dlopen(addon, process.env.AIT_NAPI_TEST_ADDON);
    const binding = addon.exports;
    const fixture = fs.realpathSync(fs.mkdtempSync(path.join(os.tmpdir(),'ait-progress-')));
    const root = path.join(fixture,'repo'); fs.mkdirSync(root);
    try {
      binding.taskWorkflowInit(root);
      binding.configureRepository(root,{agent_teams:'disabled',sprint:'off',user_name:'progress fixture'});
      const started = binding.taskStart(root,{title:'Progress',intent:'Verify native progress',local:true,editRoot:path.join(fixture,'work'),humanProgress:true});
      binding.taskFinish(root,started.task_id,{local:true});
    } finally { fs.rmSync(fixture,{recursive:true,force:true}); }
  `], { env: process.env, encoding: 'utf8' });
  assert.equal(result.status, 0, result.stderr);
  assert.match(result.stdout, /task created:/);
  assert.match(result.stdout, /worktree/);
});

test('actual addon shares Plan selection, Task start, closeout and native output', () => {
  const fixture = fs.realpathSync(fs.mkdtempSync(path.join(os.tmpdir(), 'ait-direct-workflow-')));
  const root = path.join(fixture, 'repo');
  const editRoot = path.join(fixture, 'work');
  fs.mkdirSync(root);
  try {
    const initialized = binding.taskWorkflowInit(root);
    assert.match(binding.formatInitializeRepository(initialized), /^Initialized empty AIT repository/);
    const configuration = { agent_teams: 'disabled', user_name: 'binding fixture' };
    const configured = binding.configureRepository(root, configuration);
    assert.match(binding.formatConfiguration(configured, configuration), /^ait config set\n/);
    assert.match(binding.formatConfigurationShow(binding.taskWorkflowConfigShow(root)), /^ait config show\n/);
    const source = '# Binding fixture [plan-ref: binding/fixture]\n\n- [ ] Exercise direct operations. [ref: direct/work]\n';
    fs.mkdirSync(path.join(root, 'docs'), { recursive: true });
    fs.writeFileSync(path.join(root, 'docs', 'binding.md'), source);
    const sync = binding.planSync(root, 'docs/binding.md', { local: true });
    assert.equal(binding.planSyncError(sync), null);
    assert.match(binding.formatPlanSync(sync), /^ait plan sync\n/);
    const plans = binding.planList(root, { local: true });
    const planId = plans[0].plan_id;
    assert.equal(plans[0].head_match, 'yes');
    assert.match(binding.formatPlanList(plans, { local: true }), /head_match/);
    assert.equal(binding.planShow(root, planId, { local: true }).plan_id, planId);
    assert.equal(binding.planItems(root, planId, { local: true }).items[0].plan_item_ref, 'direct/work');
    assert.ok(binding.planRevisions(root, planId, { local: true }).length > 0);
    assert.ok(binding.planInspect(root, planId, { local: true }).plan);
    assert.ok(binding.planCandidates(root, { local: true }).candidates.length > 0);
    assert.throws(() => binding.planList(root, { local: true, remoteName: 'origin' }), error => error.code === 'GenericFailure');
    const started = binding.taskStart(root, { source: 'docs/binding.md#direct/work', intent: 'Exercise direct bindings', local: true, editRoot });
    assert.equal(started.worktree.path, editRoot);
    assert.match(binding.formatTaskStart(started), /task start/);
    const compact = binding.projectTaskStart(started);
    assert.equal(compact.task_id, started.task_id);
    const authorityRoot = binding.repositoryAuthorityRoot(editRoot);
    assert.equal(authorityRoot, root);
    const finished = binding.taskFinish(editRoot, started.task_id, { local: true });
    assert.equal(binding.taskFinishExitCode(finished), 0);
    assert.match(binding.formatTaskFinish(finished, started.task_id), /finish/);
    assert.equal(fs.existsSync(editRoot), false);
    assert.throws(() => binding.projectWorkflowFinish(editRoot, finished, started.task_id), error => error.code === 'GenericFailure');
    const projected = binding.projectWorkflowFinish(authorityRoot, finished, started.task_id, { full: true });
    assert.ok(projected.identity);
    assert.equal(binding.projectStatus(binding.taskWorkflowStatus(root)).command, 'status');
    assert.match(binding.formatStatus(binding.taskWorkflowStatus(root)), /^ait status\n/);
    assert.match(binding.formatWorkflowGuide(binding.workflowGuide('finish')), /workflow guide/);
    assert.equal(binding.domainErrorExitCode(), 1);
    assert.equal(binding.formatDomainError('fixture failure'), 'Error: fixture failure');
    assert.equal(binding.encodeJson(binding.decodeJson('{"n":18446744073709551615}')), '{"n":18446744073709551615}');
    assert.equal(binding.encodeJson({ n: 4294967296 }), '{"n":4294967296}');
    assert.equal(binding.encodeJson({ n: -9223372036854775808n }), '{"n":-9223372036854775808}');
    assert.throws(() => binding.encodeJson({ nested: [18446744073709551616n] }), error =>
      error.code === 'InvalidArg' && error.message.includes('i64/u64 range'));
    assert.throws(() => binding.encodeJson({ n: 9007199254740992 }), error =>
      error.code === 'InvalidArg' && error.message.includes('BigInt'));
    const cyclic = {}; cyclic.self = cyclic;
    assert.throws(() => binding.encodeJson(cyclic), error => error.code === 'InvalidArg');
    assert.equal(binding.encodeJson({ n: 7 }), '{"n":7}');
    const budget = 9223372036854775807n;
    binding.configureRepository(root, { task_worktree_main_seed_ram_max_bytes: budget });
    assert.equal(binding.taskWorkflowConfigShow(root).task_worktree.main_seed_ram_max_bytes.value, budget);
    const unset = binding.unsetConfiguration(root, 'task-worktree-main-seed-ram-max-bytes');
    assert.match(binding.formatConfigurationUnset(unset, 'task-worktree-main-seed-ram-max-bytes'), /^ait config unset\n/);
    assert.match(binding.formatRemoteList(binding.taskWorkflowRemoteList(root)), /^name\t/);
    assert.match(binding.formatQueueSummary(binding.queueSummary(root)), /^ait queue summary\n/);
    assert.ok(binding.formatDoctorPlanAuthority(binding.doctorPlanAuthority(root)).length > 0);
    binding.configureRepository(root, { task_worktree_alias_root: path.join(fixture, 'managed') });
    const quick = binding.taskQuick(root, { intent: 'Exercise Snapshot query bindings', local: true });
    const quickRoot = quick.worktree.path;
    assert.match(binding.formatTaskQuick(quick), /\ncard: /);
    const currentWorktree = binding.taskWorkflowWorktreeGet(quickRoot, {refreshStatus:false});
    assert.equal(currentWorktree.path, quickRoot);
    assert.equal(typeof currentWorktree.name, 'string');
    assert.ok(binding.taskWorkflowWorktreeList(root, {refreshStatus:false}).some(row => row.name === currentWorktree.name));
    assert.equal(binding.taskWorkflowWorktreeDoctor(root, {refreshStatus:false}).status_mode, 'metadata');
    const shownWorktree = binding.worktreeShow(root,currentWorktree.name);
    assert.equal(shownWorktree.name,currentWorktree.name);
    assert.match(binding.formatWorktreeShow(shownWorktree),/^ait worktree show\n/);
    const openedWorktree = binding.worktreePath(root,currentWorktree.name);
    assert.equal(binding.projectWorktreePath(openedWorktree).name,currentWorktree.name);
    assert.ok(fs.existsSync(binding.formatWorktreePath(openedWorktree)));
    assert.match(binding.formatWorktreePath(openedWorktree,true),/cd/);
    assert.match(binding.formatWorktreeList(binding.taskWorkflowWorktreeList(root,{refreshStatus:false})),/^name\tpath/);
    assert.match(binding.formatWorktreeDoctor(binding.taskWorkflowWorktreeDoctor(root,{refreshStatus:false})),/doctor --refresh/);
    assert.match(binding.formatWorktreeCleanupCandidates(binding.taskWorkflowWorktreeCleanupCandidates(root,{olderThan:'7d'})),/^ait worktree cleanup-candidates\n/);
    assert.throws(() => binding.worktreeCleanup(root),/Pass --yes/);
    assert.throws(() => binding.worktreePruneStale(root),/Pass --yes/);
    assert.throws(() => binding.worktreeRemove(root,[currentWorktree.name]),/Pass --yes/);
    assert.throws(() => binding.worktreeRemove(root,[],{dryRun:true}),/Provide one or more/);
    assert.throws(() => binding.worktreeRemove(root,[currentWorktree.name],{allStale:true,dryRun:true}),/Choose either/);
    const cleaned = binding.worktreeCleanup(root,{olderThan:'7d',dryRun:true,limit:1n});
    assert.equal(cleaned.dry_run,true); assert.match(binding.formatWorktreeCleanup(cleaned),/^ait worktree cleanup\n/);
    const pruned = binding.worktreePruneStale(root,{dryRun:true});
    assert.equal(pruned.dry_run,true); assert.match(binding.formatWorktreePruneStale(pruned),/^ait worktree prune-stale\n/);
    const removedStale = binding.worktreeRemove(root,[],{allStale:true,dryRun:true});
    assert.equal(binding.formatWorktreeRemove(removedStale,true,true,false),binding.formatWorktreePruneStale(removedStale));
    assert.throws(() => binding.worktreeSync(root,{allWorktrees:true,name:currentWorktree.name}),/Choose either/);
    assert.throws(() => binding.worktreeSync(root,{allWorktrees:true,lineName:'main'}),/--line cannot/);
    assert.throws(() => binding.worktreeRebase(root,{continueRebase:true,abortRebase:true}),/Choose either/);
    assert.throws(() => binding.worktreeRebase(root,{continueRebase:true,dryRun:true}),/--dry-run cannot/);
    assert.equal(binding.worktreeSyncExitCode({ok:false},true),2);
    assert.equal(binding.worktreeSyncExitCode({ok:false},false),0);
    binding.taskWorkflowWorktreeTouchUsage(quickRoot);
    assert.ok(binding.taskWorkflowWorktreeCleanupCandidates(root, {olderThan:'7d',includeProtected:true}).inspected_count >= 1);
    assert.throws(() => binding.taskWorkflowWorktreeGet(root, {name:'absent'}), error => error.code === 'GenericFailure');
    assert.throws(() => binding.taskWorkflowWorktreeCleanup(root, {limit:1.5,dryRun:true}), /native-size integer/);
    fs.writeFileSync(path.join(quickRoot, 'changed.txt'), 'actual addon capture\n');
    const dirty = binding.workspaceDiff(quickRoot, ['changed.txt']);
    const worktreeStatus = binding.taskWorkflowWorktreeStatus(quickRoot);
    assert.match(binding.formatWorktreeStatus(worktreeStatus,true),/changed: changed.txt/);
    assert.equal(binding.renderWorkspaceDiffNames(dirty), 'changed.txt\n');
    assert.match(binding.renderWorkspaceDiffStat(dirty), /changed.txt\t\+/);
    assert.match(binding.renderWorkspaceDiffText(dirty), /changed.txt/);
    const captured = binding.snapshotCapture(quickRoot, quick.task_id, { message: 'direct capture' });
    assert.ok(captured.snapshot_id);
    const storageStats = binding.gcStats(quickRoot);
    assert.ok(storageStats.pack_count > 0);
    assert.match(binding.formatGcStats(storageStats),/^ait gc stats\n/);
    const storageValidation = binding.gcValidate(quickRoot);
    assert.equal(binding.gcValidateExitCode(storageValidation),0);
    assert.match(binding.formatGcValidate(storageValidation),/^ait gc validate\n/);
    assert.equal(binding.gcValidateExitCode({needs_attention:true}),1);
    assert.throws(() => binding.gcValidateExitCode({}),/boolean needs_attention/);
    const prunePreview = binding.gcPrune(quickRoot);
    assert.equal(prunePreview.applied,false);
    assert.match(binding.formatGcPrune(prunePreview),/^ait gc prune\n/);
    const prunedStorage = binding.gcPrune(quickRoot,true);
    assert.equal(prunedStorage.applied,true);
    assert.equal(binding.taskWorkflowSnapshotExists(quickRoot,captured.snapshot_id),true);
    const restoredWorktree = binding.worktreeRestore(quickRoot,{snapshotId:captured.snapshot_id,dryRun:true});
    assert.equal(restoredWorktree.applied,false);
    assert.match(binding.formatWorktreeRestore(restoredWorktree),/^ait worktree restore\n/);
    const synchronized = binding.worktreeSync(quickRoot,{dryRun:true});
    assert.match(binding.formatWorktreeSync(synchronized),/^ait worktree sync\n/);
    const rebased = binding.worktreeRebase(quickRoot,{ontoLine:'main',dryRun:true});
    assert.match(binding.formatWorktreeRebase(rebased),/^ait worktree rebase\n/);
    assert.match(binding.formatSnapshotCapture(captured), /^ait snapshot create\n/);
    const cleanDiff = binding.workspaceDiff(quickRoot);
    assert.equal(binding.renderWorkspaceDiffNames(cleanDiff), '');
    assert.equal(binding.renderWorkspaceDiffStat(cleanDiff), '');
    assert.equal(binding.renderWorkspaceDiffText(cleanDiff), '');
    const blame = binding.taskWorkflowBlame(quickRoot, 'changed.txt', {snapshotId:captured.snapshot_id,line:1n});
    assert.equal(blame.resolved_snapshot_id, captured.snapshot_id);
    assert.equal(blame.lines[0].task_id, quick.task_id);
    assert.match(binding.formatBlame(blame), /changed.txt/);
    assert.throws(() => binding.taskWorkflowBlame(quickRoot,'changed.txt',{line:0}), /must be positive/);
    assert.equal(binding.taskWorkflowSnapshotExists(quickRoot, captured.snapshot_id), true);
    assert.equal(binding.taskWorkflowSnapshotExists(quickRoot, 'SNP-000000000000'), false);
    assert.ok(binding.encodeJson(binding.taskWorkflowSnapshotChain(quickRoot, captured.snapshot_id)).includes(captured.snapshot_id));
    const bytes = Buffer.from([0,255,10,13,128,42]);
    const blob = binding.taskWorkflowBlobEnsureBytes(quickRoot, bytes, 'binary.fixture');
    assert.deepEqual(binding.taskWorkflowBlobReadBytes(quickRoot, blob), bytes);
    assert.throws(() => binding.taskWorkflowBlobReadBytes(quickRoot, 'BLB-00000000000000000000'), error => error.code === 'GenericFailure');
    const lock = new binding.WorkspaceCommandLock(quickRoot, 'direct SDK lock fixture');
    assert.equal(lock.isClosed(), false);
    assert.equal(lock.metadata().command, 'direct SDK lock fixture');
    assert.equal(lock.metadata().lock_path, binding.workspaceCommandLockPath(quickRoot));
    lock.release(); lock.close(); assert.equal(lock.isClosed(), true);
    const inspection = binding.inspectSnapshot(quickRoot, captured.snapshot_id, true);
    assert.equal(inspection.payload.snapshot_id, captured.snapshot_id);
    assert.match(binding.formatSnapshotShow(inspection), /changed paths/);
    assert.match(binding.formatSnapshotShow(inspection, true), /\nfiles\n/);
    const snapshotRows = binding.taskWorkflowSnapshotList(quickRoot);
    assert.ok(binding.projectSnapshotList(snapshotRows).length > 0);
    assert.match(binding.formatSnapshotList(snapshotRows), /^snapshot_id\t/);
    const taskRows = binding.taskWorkflowTaskList(root, { local: true, all: true });
    assert.ok(binding.projectTaskList(taskRows).some(task => task.task_id === quick.task_id));
    assert.match(binding.formatTaskList(taskRows, { local: true }), /^task_id\t/);
    assert.match(binding.formatTaskShow(binding.taskWorkflowTaskShow(root, quick.task_id, { local: true })), /^ait task show\n/);
    const parent = captured.parent_snapshot_id;
    assert.ok(parent);
    const currentTag = binding.tagCreate(quickRoot, 'binding-current', 'Current captured source');
    assert.equal(currentTag.snapshot_id, captured.snapshot_id);
    assert.match(binding.formatTagCreate(currentTag), /^ait tag create\n/);
    binding.tagCreate(root, 'binding-parent', 'Original baseline', parent);
    assert.equal(binding.resolveSnapshotRef(root, 'binding-current'), captured.snapshot_id);
    assert.equal(binding.tagShow(root, 'binding-parent').snapshot_id, parent);
    assert.match(binding.formatTagShow(binding.tagShow(root, 'binding-parent')), /^ait tag show\n/);
    assert.match(binding.formatTagList(binding.tagList(root)), /binding-current/);
    assert.throws(() => binding.tagCreate(root, 'binding-parent', 'Duplicate', parent), error => error.code === 'GenericFailure');
    assert.equal(binding.taskWorkflowWorkspaceDelta(quickRoot, captured.snapshot_id).clean, true);
    const restorePreview = {targetSnapshotId:parent,baselineSnapshotId:captured.snapshot_id,dryRun:true,force:true};
    assert.equal(binding.taskWorkflowWorkspaceRestore(quickRoot, restorePreview).applied, false);
    assert.equal(binding.taskWorkflowWorkspaceRestorePaths(quickRoot, ['changed.txt'], restorePreview).applied, false);
    assert.equal(fs.readFileSync(path.join(quickRoot, 'changed.txt'),'utf8'), 'actual addon capture\n');
    const nativeDiff = binding.snapshotDiff(quickRoot, parent, captured.snapshot_id, {includeText:true,maxBytes:65536n});
    assert.ok(nativeDiff.files.some(file => file.path === 'changed.txt'));
    assert.match(binding.formatSnapshotDiff(nativeDiff), /changed.txt/);
    assert.deepEqual(binding.taskWorkflowSnapshotDiff(quickRoot, parent, captured.snapshot_id), binding.snapshotDiff(quickRoot, parent, captured.snapshot_id));
    assert.deepEqual(binding.snapshotDiff(quickRoot, 'binding-parent', 'binding-current'), binding.snapshotDiff(quickRoot, parent, captured.snapshot_id));
    assert.throws(() => binding.snapshotDiff(quickRoot, parent, captured.snapshot_id, {maxBytes:1.5}), /native-size integer/);
    const reverted = binding.snapshotRevert(quickRoot, 'binding-current', {dryRun:true});
    assert.equal(reverted.applied, false);
    assert.match(binding.formatSnapshotRevert(reverted), /^ait snapshot revert\n/);
    const replayed = binding.snapshotReplay(quickRoot, 'binding-current', null, {dryRun:true,force:true});
    assert.equal(replayed.applied, false);
    assert.match(binding.formatSnapshotReplay(replayed), /^ait snapshot replay\n/);
    const selectedLines = binding.taskWorkflowSnapshotRestoreLines(quickRoot, captured.snapshot_id, 'changed.txt', {line:1n});
    assert.equal(selectedLines.applied, false);
    assert.match(binding.formatSnapshotRestoreLines(selectedLines), /^ait snapshot restore-lines\n/);
    assert.match(binding.formatTagDelete(binding.tagDelete(root,'binding-current')), /^ait tag delete\n/);
    binding.tagDelete(root,'binding-parent');
    assert.deepEqual(binding.tagList(root), []);
    const ancestor = binding.snapshotIsAncestor(quickRoot, parent, captured.snapshot_id);
    assert.equal(ancestor.exitCode, 0);
    assert.equal(binding.formatSnapshotIsAncestor(ancestor.payload), 'ancestor');
    const reverse = binding.snapshotIsAncestor(quickRoot, captured.snapshot_id, parent);
    assert.equal(reverse.exitCode, 1);
    assert.equal(binding.formatSnapshotIsAncestor(reverse.payload), 'not ancestor');
    const base = binding.snapshotMergeBase(quickRoot, parent, captured.snapshot_id, true);
    assert.equal(base.exitCode, 0);
    assert.equal(binding.formatSnapshotMergeBase(base.payload), parent);
    const history = binding.snapshotAncestry(quickRoot, captured.snapshot_id, { maxDepth: 1n, limit: 2n });
    assert.match(binding.formatSnapshotAncestry(history), /^ait snapshot ancestry\n/);
    assert.equal(binding.snapshotQueryErrorExitCode(), 2);
    assert.equal(binding.formatSnapshotQueryError('lookup failed'), 'Error: lookup failed');
    assert.throws(() => binding.snapshotIsAncestor(quickRoot, 'missing-snapshot', parent), error => error.code === 'GenericFailure');
    assert.throws(() => binding.snapshotAncestry(quickRoot, parent, { limit: 0 }), error => error.code === 'GenericFailure');
    assert.ok(binding.taskAudit(quickRoot, quick.task_id, { local: true }).task);
    binding.taskWorkflowLineSetHead(quickRoot, binding.taskWorkflowStatus(quickRoot).current_line, captured.snapshot_id);
    const abandoned = binding.taskAbandon(quickRoot, quick.task_id, { local: true });
    assert.equal(abandoned.exitCode, 0);
    assert.match(binding.formatTaskAbandon(abandoned.payload, quick.task_id), /^ait task abandon\n/);
    assert.equal(fs.existsSync(quickRoot), false);
    const line = binding.taskWorkflowLineCreate(root, 'binding/source', { fromSnapshot: parent });
    assert.match(binding.formatLineCreate(line), /^ait line create\n/);
    const inventory = binding.lineInventory(root);
    assert.ok(inventory.payload.some(line => line.line_name === 'binding/source'));
    assert.match(binding.formatLineList(inventory), /^line_name\t/);
    assert.equal(binding.lineShow(root, 'binding/source').line_name, 'binding/source');
    assert.ok(binding.taskWorkflowLineList(root).some(line => line.line_name === 'binding/source'));
    assert.equal(binding.taskWorkflowLineShow(root, 'binding/source').line_name, 'binding/source');
    assert.match(binding.formatLineShow(binding.lineShow(root, 'binding/source')), /^ait line show\n/);
    assert.throws(() => binding.taskWorkflowLineSetHead(root, 'binding/source', parent), error =>
      error.code === 'GenericFailure' && error.message.includes('authoring workspace'));
    const renamed = binding.taskWorkflowLineRename(root, 'binding/source', 'binding/renamed');
    assert.match(binding.formatLineRename(renamed), /^ait line rename\n/);
    const switched = binding.taskWorkflowLineSwitch(root, 'binding/renamed');
    assert.match(binding.formatLineSwitch(switched), /^ait line switch\n/);
    binding.taskWorkflowLineSwitch(root, 'main');
    const archived = binding.taskWorkflowLineArchive(root, 'binding/renamed');
    assert.match(binding.formatLineArchive(archived), /^ait line archive\n/);
    assert.throws(() => binding.taskWorkflowLineDelete(root, 'binding/renamed'), error => error.code === 'GenericFailure');
    const deleted = binding.taskWorkflowLineDelete(root, 'binding/renamed', { confirm: true });
    assert.match(binding.formatLineDelete(deleted), /^ait line delete\n/);
    assert.throws(() => binding.lineMerge(root, { continueMerge: true, abortMerge: true }), error => error.code === 'GenericFailure');
    const cleanup = binding.taskWorkflowLineCleanup(root, { idleFor: '7d', limit: 2n, includeProtected: true });
    assert.match(binding.formatLineCleanup(cleanup, '7d', { limit: 2n, includeProtected: true }), /^ait line cleanup\n/);
  } finally {
    fs.rmSync(fixture, { recursive: true, force: true });
  }
});

test('actual addon preserves Task-scoped Change operations and native text', () => {
  const fixture = fs.realpathSync(fs.mkdtempSync(path.join(os.tmpdir(), 'ait-direct-change-')));
  const root = path.join(fixture, 'repo'); fs.mkdirSync(root);
  try {
    binding.taskWorkflowInit(root);
    binding.configureRepository(root, {agent_teams:'disabled',sprint:'on',user_name:'binding fixture',task_worktree_alias_root:path.join(fixture,'managed')});
    const task = binding.taskQuick(root, {intent:'Exercise explicit revision operations',local:true});
    const work = task.worktree.path;
    fs.writeFileSync(path.join(work,'code.rs'), 'fn before_review() {}\n');
    const fork = binding.snapshotCapture(work, task.task_id, {message:'Snapshot to revise'});
    const change = binding.changeCreate(root, task.task_id, 'Revise the captured source', {local:true,forkSnapshot:fork.snapshot_id});
    assert.equal(change.task_id, task.task_id);
    assert.match(binding.formatChangeCreate(change), /^ait change create\n/);
    const shown = binding.changeShow(root, task.task_id, {local:true});
    assert.equal(shown.change_id, change.change_id);
    assert.match(binding.formatChangeShow(shown), /^ait change show\n/);
    const reference = shown.change_ref;
    fs.writeFileSync(path.join(work,'code.rs'), 'fn after_review() {}\n');
    binding.snapshotCapture(work, reference, {message:'Capture the explicit revision'});
    const reverted = binding.changeRevert(work, task.task_id, {local:true,dryRun:true});
    assert.equal(reverted.applied, false);
    assert.match(binding.formatChangeRevert(reverted), /^ait change revert\n/);
    assert.equal(fs.readFileSync(path.join(work,'code.rs'),'utf8'), 'fn after_review() {}\n');
    const replayed = binding.changeReplay(work, task.task_id, null, {local:true,dryRun:true,force:true});
    assert.equal(replayed.applied, false);
    assert.match(binding.formatChangeReplay(replayed), /^ait change replay\n/);
    const rows = binding.taskWorkflowChangeList(root, {local:true});
    assert.equal(binding.projectChangeList(rows).length, 1);
    assert.match(binding.formatChangeList(rows, {local:true}), /^task_id\t/);
    assert.throws(() => binding.changeShow(root, 'LT-9999', {local:true}), error => error.code === 'GenericFailure');
    const closed = binding.changeClose(work, reference, {local:true});
    assert.equal(closed.status, 'archived');
    assert.ok(closed.automatic_reconciliation);
    assert.match(binding.formatChangeClose(closed), /^ait change close\n/);
    binding.taskAbandon(root, task.task_id, {local:true});
  } finally { fs.rmSync(fixture, {recursive:true,force:true}); }
});

test('actual addon preserves the Stash lifecycle, conflict errors and native text', () => {
  const fixture = fs.realpathSync(fs.mkdtempSync(path.join(os.tmpdir(), 'ait-direct-stash-')));
  const root = path.join(fixture, 'repo'); fs.mkdirSync(root);
  try {
    binding.taskWorkflowInit(root);
    binding.configureRepository(root, {agent_teams:'disabled',sprint:'on',user_name:'binding fixture',task_worktree_alias_root:path.join(fixture,'managed')});
    const task = binding.taskQuick(root, {intent:'Exercise direct Stash operations',local:true});
    const work = task.worktree.path; const file = path.join(work,'work.txt');
    fs.writeFileSync(file, 'base\n'); binding.snapshotCapture(work, task.task_id, {message:'Stash baseline'});
    assert.throws(() => binding.stashSave(work), /already clean/);
    fs.writeFileSync(file, 'parked\n');
    const saved = binding.stashSave(work, {message:'Parked work'});
    assert.equal(saved.workspace_cleared, true);
    assert.equal(fs.readFileSync(file,'utf8'), 'base\n');
    assert.match(binding.formatStashSave(saved), /^ait stash save\n/);
    const rows = binding.taskWorkflowStashList(work);
    assert.equal(rows[0].stash_id, saved.stash_id);
    assert.match(binding.formatStashList(rows), /Parked work/);
    assert.match(binding.formatStashShow(binding.taskWorkflowStashShow(work,saved.stash_id)), /^ait stash show\n/);
    const applied = binding.stashApply(work, saved.stash_id);
    assert.equal(applied.applied, true); assert.equal(applied.dropped, false);
    assert.match(binding.formatStashApply(applied), /^ait stash apply\n/);
    assert.throws(() => binding.stashPop(work,saved.stash_id), /Re-run with --force/);
    assert.equal(binding.taskWorkflowStashList(work).length, 1);
    const popped = binding.stashPop(work,saved.stash_id,true);
    assert.equal(popped.dropped,true); assert.match(binding.formatStashPop(popped), /^ait stash pop\n/);
    const kept = binding.stashSave(work,{message:'Keep workspace',keepWorkspace:true});
    assert.equal(kept.workspace_cleared,false);
    const dropped = binding.stashDrop(work,kept.stash_id);
    assert.equal(dropped.dropped,true); assert.match(binding.formatStashDrop(dropped), /^ait stash drop\n/);
    assert.deepEqual(binding.taskWorkflowStashList(work),[]);
    assert.equal(fs.readFileSync(file,'utf8'),'parked\n');
    binding.taskAbandon(root,task.task_id,{local:true});
  } finally { fs.rmSync(fixture,{recursive:true,force:true}); }
});

test('actual addon retains artifact matching, filesystem guards and exact JSON/binary I/O', () => {
  const fixture=fs.realpathSync(fs.mkdtempSync(path.join(os.tmpdir(),'ait-direct-filesystem-')));
  const root=path.join(fixture,'repo');fs.mkdirSync(root);
  try {
    const body='# SDK artifact\n';const blob=binding.artifactBlobId(body);
    const generic={plan_id:'PR-1',title:'SDK',status:'draft',publication_state:'published',published_head_revision_id:'rev-1',head_revision:{artifact_path:'docs/sdk.md',artifact_selector:null,artifact_heading:'SDK',artifact_blob_id:blob,items:[],publication_state:'published'}};
    const selected={...generic,plan_id:'PR-2',head_revision:{...generic.head_revision,artifact_selector:'sdk/root'}};
    const done={...generic,plan_id:'PR-3',status:'archived'};const plans=[generic,selected,done];
    assert.match(blob,/^BLB-/);
    assert.deepEqual(binding.planArtifactIdentity('docs/sdk.md',' sdk/root '),{artifact_path:'docs/sdk.md',artifact_selector:'sdk/root'});
    assert.equal(binding.planArtifactIdentityLabel('docs/sdk.md','sdk/root'),'docs/sdk.md [sdk/root]');
    assert.deepEqual(binding.artifactCandidatesOpen(plans).map(row=>row.plan_id),['PR-1','PR-2']);
    assert.equal(binding.indexPlansByArtifactPath(plans)['docs/sdk.md'].length,3);
    assert.equal(binding.indexPlansByArtifactIdentity(plans).length,2);
    assert.deepEqual(binding.openGenericPlansMatchingBlobId(plans,blob).map(row=>row.plan_id),['PR-1']);
    assert.deepEqual(binding.openPlansMatchingSelector(plans,'sdk/root').map(row=>row.plan_id),['PR-2']);
    assert.equal(binding.localPlanFullyPublished(generic),true);
    assert.equal(binding.planHeadsEquivalent(generic,{...generic,plan_id:'other'}),true);
    const artifact={artifact_path:'docs/sdk.md',artifact_selector:null,artifact_heading:'SDK',artifact_body:body,items:[]};
    assert.equal(binding.planMatchesSyncArtifact(generic,artifact),true);
    assert.equal(binding.planMatchesSyncArtifact({...generic,title:'Other'},artifact),false);
    assert.equal(binding.planMatchesSyncArtifact({...generic,title:'Other'},artifact,false),true);
    assert.throws(() => binding.indexPlansByArtifactPath({}),error=>error.code==='GenericFailure');
    fs.mkdirSync(path.join(root,'docs'));fs.mkdirSync(path.join(root,'target'));
    fs.writeFileSync(path.join(root,'docs','spec.md'),body);fs.writeFileSync(path.join(root,'src.rs'),'fn fixture() {}\n');
    fs.writeFileSync(path.join(root,'target','generated.txt'),'generated');
    assert.equal(binding.planFilesystemNormalizeMarkdownArtifactPath('docs\\spec.MD'),'docs/spec.MD');
    assert.equal(binding.planFilesystemIsMarkdownArtifactPath('docs/spec.MD'),true);
    assert.equal(binding.planFilesystemIsLineageOnlyMarkdownArtifactPath('docs/spec.md'),true);
    assert.equal(binding.planFilesystemPathIsProjectedOutForWorkspace(root,'docs/data.txt',true),true);
    assert.equal(binding.planFilesystemWorkspacePathIsIgnored(root,'target/generated.txt','target/\n'),true);
    const options={ignoreRulesText:'target/\n',runtimeRoot:path.join(fixture,'runtime')};
    const paths=binding.planFilesystemListVisibleWorkspacePaths(root,options);
    assert.ok(paths.includes('src.rs'));assert.equal(paths.includes('target/generated.txt'),false);
    assert.ok(binding.planFilesystemListVisibleMarkdownArtifactPaths(root,options).includes('docs/spec.md'));
    const textPath=path.join(root,'text.txt');fs.writeFileSync(textPath,'first\r\nsecond\r');
    assert.equal(binding.planFilesystemReadUtf8TextFile(textPath),'first\nsecond\n');
    const bytes=Buffer.from([0,255,10,128]);const binaryPath=path.join(root,'payload.bin');fs.writeFileSync(binaryPath,bytes);
    assert.deepEqual(binding.planFilesystemReadBinaryFile(binaryPath),bytes);
    const archive=path.join(root,'fixture.zip');
    fs.writeFileSync(archive,Buffer.from('UEsDBBQAAAAAAAAAIQBbgIuIBAAAAAQAAAALAAAAcGF5bG9hZC5iaW4A/wqAUEsBAhQDFAAAAAAAAAAhAFuAi4gEAAAABAAAAAsAAAAAAAAAAAAAAIABAAAAAHBheWxvYWQuYmluUEsFBgAAAAABAAEAOQAAAC0AAAAAAA==','base64'));
    assert.equal(binding.planFilesystemZipArchiveHasMember(archive,'payload.bin'),true);
    assert.equal(binding.planFilesystemZipArchiveHasMember(archive,'missing'),false);
    assert.deepEqual(binding.planFilesystemReadZipArchiveMember(archive,'payload.bin'),bytes);
    assert.throws(() => binding.planFilesystemReadZipArchiveMember(archive,'missing'),error=>error.code==='GenericFailure');
    assert.equal(binding.planFilesystemResolveRepoArtifactPath(root,'docs/spec.md').artifact_path,'docs/spec.md');
    assert.equal(binding.planFilesystemResolveRepoArtifactPath(root,'docs/absent.md',true).artifact_path,'docs/absent.md');
    assert.throws(() => binding.planFilesystemResolveRepoArtifactPath(root,'../outside.md',true),/inside the repository root/);
    assert.throws(() => binding.planFilesystemResolveRepoArtifactPath(root,'.ait/runtime.md',true),/runtime metadata/);
    for (const [write,read,label] of [[binding.repoStateWriteRepoConfigJson,binding.repoStateReadRepoConfigJson,'repo'],[binding.repoStateWriteWorktreeConfigJson,binding.repoStateReadWorktreeConfigJson,'worktree'],[binding.repoStateWriteWorktreeMetadataJson,binding.repoStateReadWorktreeMetadataJson,'metadata']]) {
      const file=path.join(root,label+'.json');assert.equal(read(file),null);
      const payload={label,maximum:18446744073709551615n,minimum:-9223372036854775808n};write(file,payload);
      assert.deepEqual(read(file),payload);assert.deepEqual(binding.planFilesystemReadJsonFile(file),payload);
    }
    assert.throws(() => binding.planFilesystemReadJsonFile(path.join(root,'missing.json')),error=>error.code==='GenericFailure');
  } finally {fs.rmSync(fixture,{recursive:true,force:true});}
});

test('actual addon preserves native Plan dispatch, link indexes and candidate decisions', () => {
  const items=[
    {plan_item_ref:'demo/linked',text:'Linked',checkbox_state:'open',heading_path:['Demo'],line_number:10},
    {plan_item_ref:'demo/free',text:'Free',checkbox_state:'open',heading_path:['Demo'],line_number:11},
    {plan_item_ref:null,text:'Missing ref',checkbox_state:'open',heading_path:['Demo'],line_number:12},
    {plan_item_ref:'demo/done',text:'Done',checkbox_state:'done',heading_path:['Demo'],line_number:13},
  ];
  const plan={plan_id:'PR-1',title:'Demo',status:'draft',repo_name:'fixture',publication_state:'published',published_plan_id:'PR-1',published_head_revision_id:'rev-1',head_revision_id:'rev-2',head_revision:{plan_revision_id:'rev-2',revision_number:2,artifact_path:'docs/demo.md',publication_state:'local_draft',items}};
  const tasks=[{task_id:'LT-1',title:'Linked task',status:'active',planning_state:'planned',origin_plan_revision_id:'rev-1',plan_id:'PR-1',plan_item_ref:'demo/linked'}];
  const payload=binding.planItemsPayload(plan);
  assert.equal(payload.identity_only,true); assert.equal(payload.dispatch_validation_required,true); assert.equal(payload.item_count,4);
  assert.equal(binding.planItemsPayload({head_revision:{plan_revision_id:'empty'}}).item_count,0);
  const shadow=binding.localPlanPublishShadow(plan); assert.equal(shadow.unpublished_head,true); assert.equal(shadow.head_revision_number,2);
  assert.equal(binding.localPlanPublishShadow(),null);
  const summary=binding.planDispatchSummary(plan,{tasks});
  assert.equal(summary.linked_task_count,1); assert.equal(summary.taskable_item_count,1); assert.equal(summary.unref_open_item_count,1);
  assert.equal(summary.items.find(row=>row.plan_item_ref==='demo/linked').taskable_blocker,'linked_task_exists');
  assert.equal(binding.computeTaskableItems(plan,{tasks})[0].plan_item_ref,'demo/free');
  assert.equal(binding.validateDispatchLegality(plan,'demo/free',{tasks}).taskable,true);
  assert.equal(binding.validateDispatchLegality(plan,'demo/linked',{tasks}).taskable,false);
  assert.equal(binding.validateDispatchLegality(plan,'absent',{tasks}).taskable_blocker,'plan_item_not_found');
  const indexes=binding.planTaskLinkIndexes(tasks);
  assert.equal(indexes.by_item[0].plan_id,'PR-1'); assert.equal(indexes.by_item[0].plan_item_ref,'demo/linked');
  assert.equal(indexes.by_item[0].tasks[0].task_id,'LT-1'); assert.equal(indexes.by_plan['PR-1'][0].task_id,'LT-1');
  const candidates=binding.planCandidatesPayload([summary],{scope:'local',repoName:'fixture'});
  assert.equal(candidates.summary.candidate_plan_count,1); assert.equal(candidates.candidates[0].taskable_item_count,1);
  assert.equal(binding.planCandidatesPayload([{}],{includeAll:false}).candidates.length,0);
  assert.equal(binding.planCandidatesPayload([{}],{includeAll:true}).candidates.length,1);
  assert.throws(() => binding.planDispatchSummary(plan,{tasks:{}}),/must be a list/);
  assert.throws(() => binding.planCandidatesPayload([{...summary,item_count:1.5}]),/native-size integer/);
});

test('actual addon shares pure Plan parsing, prune decisions and byte-preserving manifest diffs', () => {
  const markdown='# SDK [plan-ref: sdk/plan]\n\n- [ ] Capture SDK. [ref: sdk/work]\n\n```\n# Ignored [plan-ref: ignored]\n- [ ] Ignored. [ref: ignored/work]\n```\n';
  const parsed=binding.parsePlanMarkdown(markdown);
  assert.equal(parsed.markdown_text,markdown); assert.equal(parsed.plan_ref_count,1); assert.equal(parsed.item_count,1);
  assert.equal(parsed.items[0].plan_item_ref,'sdk/work');
  assert.deepEqual(binding.extractPlanItems(markdown),parsed.items);
  assert.deepEqual(binding.listPlanSectionRefs(markdown),parsed.plan_refs);
  assert.deepEqual(binding.extractPlanSection(markdown,'sdk/plan').items,parsed.items);
  assert.equal(binding.extractPlanSection(markdown,'absent'),null);
  assert.deepEqual(binding.findPlanItem(markdown,'sdk/work'),parsed.items[0]);
  assert.equal(binding.findPlanItem(markdown,'absent'),null);
  assert.deepEqual(binding.normalizePlanItems(parsed.items),parsed.items);
  assert.deepEqual(binding.findPlanItemInItems(parsed.items,'sdk/work'),parsed.items[0]);
  assert.equal(binding.findPlanItemInItems(parsed.items,'absent'),null);
  assert.throws(() => binding.normalizePlanItems([{plan_item_ref:'work',checkbox_state:'invalid'}]),/checkbox/i);
  assert.deepEqual(binding.normalizeTaskWorkflowHttpCompatibility('{"ready":true}'),{ready:true});
  assert.deepEqual(binding.normalizeTaskWorkflowHttpReadiness('{"ready":true}'),{ready:true});
  assert.throws(() => binding.normalizeTaskWorkflowHttpCompatibility('{'),/invalid JSON/);
  assert.throws(() => binding.normalizeTaskWorkflowHttpReadiness('[]'),/must be an object/);
  assert.deepEqual(binding.extractPlanRefs(parsed),{plan_ref_count:1,plan_refs:parsed.plan_refs});
  assert.throws(() => binding.extractPlanRefs({plan_refs:[{}]}),/include plan_ref/);
  assert.throws(() => binding.extractPlanRefs({...parsed,items:[{}]}),/include plan_item_ref/);
  assert.throws(() => binding.extractPlanRefs({...parsed,plan_ref_count:1.5}),/native-size integer/);
  const prune=binding.computeSyncPruneDecisions('directory',['a.md','b.md','a.md'],['a.md']);
  assert.deepEqual(prune.retained_paths,['a.md']); assert.deepEqual(prune.prune_paths,['b.md']);
  assert.equal(prune.tracked_artifact_count,2); assert.equal(prune.prune_count,1);
  assert.throws(() => binding.computeSyncPruneDecisions('unsupported',[],[]),/Unsupported sync prune scope/);
  const oldFiles={'a.txt':{path:'a.txt',blob_id:'BLB-OLD',size_bytes:10,mode:'0o644'}};
  const newFiles={'a.txt':{path:'a.txt',blob_id:'BLB-NEW',size_bytes:10,mode:'0o644'}};
  const identity={oldSnapshotId:'S1',newSnapshotId:'S2'};
  assert.deepEqual(binding.diffSnapshotManifests(oldFiles,newFiles,identity).modified,['a.txt']);
  const bytes={'BLB-OLD':Buffer.from('hello\nfoo\n'),'BLB-NEW':Buffer.from('hello\nbar\n')};
  const diff=binding.snapshotDiffFromManifests(oldFiles,newFiles,{...identity,blobBytesById:bytes,includeText:true,maxBytes:1000n});
  assert.equal(diff.summary.old_snapshot_id,'S1'); assert.equal(diff.files[0].diff.status,'text'); assert.match(diff.files[0].diff.text,/\+bar/);
  assert.equal(binding.snapshotDiffFromManifests(oldFiles,newFiles,{blobBytesById:bytes,includeText:true,maxBytes:4}).files[0].diff.status,'too_large');
  assert.equal(binding.snapshotDiffFromManifests(oldFiles,newFiles,{blobBytesById:{...bytes,'BLB-NEW':Buffer.from([0,255,1])},includeText:true}).files[0].diff.status,'binary');
  assert.throws(() => binding.snapshotDiffFromManifests(oldFiles,newFiles,{maxBytes:1.5}),/native-size integer/);
});

test('actual addon preserves retained workflow identity, status and result SDK contracts', () => {
  assert.equal(binding.normalizeTaskStatus('completed'),'completed');
  assert.equal(binding.normalizeTaskStatus(),null);
  assert.equal(binding.taskStatusDetails('completed').closed,true);
  assert.equal(binding.normalizePublicationState('local_draft'),'local_draft');
  assert.equal(binding.publicationStateHasUnpublishedHead('local_draft'),true);
  assert.throws(() => binding.normalizeTaskStatus('unsupported'),error=>error.code==='GenericFailure');
  assert.equal(binding.normalizeOptionalText('  retained SDK  '),'retained SDK');
  assert.equal(binding.normalizeOptionalText('  '),null);
  assert.equal(binding.normalizeIdNamespacePrefix(' lc '),'LC');
  assert.throws(() => binding.normalizeIdNamespacePrefix('bad!'),/ASCII letters or digits/);
  assert.equal(binding.workflowIdToken('T','LC'),'LCT');
  assert.ok(binding.workflowIdTokens('T',{namespacePrefix:'LC'}).includes('LCT'));
  assert.ok(binding.workflowIdNamespacePrefixCandidates({namespacePrefix:'LC'}).includes('LC'));
  assert.equal(binding.workflowOriginNamespacePrefix('L','C'),'LC');
  const sequence=binding.generateNamespacedSequenceId('T',7n,{namespacePrefix:'LC'});
  assert.equal(sequence,'LCT-0007');
  assert.equal(binding.workflowIdNamespacePrefixForValue('T',sequence,{namespacePrefix:'LC'}),'LC');
  assert.equal(binding.workflowIdMatches('T',sequence,{namespacePrefix:'LC'}),true);
  assert.equal(binding.workflowIdMatchesAnyNamespacePrefix('T',sequence,{namespacePrefix:'LC'}),true);
  assert.equal(binding.generateNamespacedSequenceId('T',9223372036854775807n,{namespacePrefix:'LC'}),'LCT-9223372036854775807');
  assert.throws(() => binding.generateNamespacedSequenceId('T',9223372036854775808n),/signed 64-bit integer/);
  assert.throws(() => binding.generateNamespacedSequenceId('T',1,{width:1.5}),/native-size integer/);
  assert.equal(binding.derivePatchsetId('LCC-0007',2n,'LC'),'LCP-0007-2');
  assert.throws(() => binding.derivePatchsetId('invalid',1),/Unsupported change id/);
  assert.match(binding.utcNow(),/^\d{4}-\d{2}-\d{2}T/);
  assert.match(binding.generateWorkflowId('SDK'),/^SDK-[0-9A-Z]{26}$/);
  const generated=binding.generateNamespacedWorkflowId('T','LC');
  assert.equal(binding.workflowIdMatches('T',generated,{namespacePrefix:'LC'}),true);
  assert.deepEqual(binding.workflowSuccessEnvelope('sdk',' value '),{ok:true,kind:'sdk',value:'value',error:null});
  assert.deepEqual(binding.workflowErrorEnvelope('sdk','invalid','message',' detail '),{ok:false,kind:'sdk',value:null,error:{code:'invalid',message:'message',detail:'detail'}});
  assert.throws(() => binding.workflowSuccessEnvelope('  '),/Workflow kind is required/);
});

test('actual addon shares retained policy, provenance, runtime and ref-name SDK helpers', () => {
  assert.ok(binding.policyProfileNames().includes('prototype'));
  assert.ok(binding.authorModeValues().includes('ai_only_experimental'));
  assert.ok(binding.policyContentClassValues().includes('docs_only'));
  assert.ok(binding.policyAuthorClassValues().includes('ai_related'));
  assert.equal(binding.normalizeAuthorMode(' ai_only_experimental '),'ai_only_experimental');
  assert.throws(() => binding.normalizeAuthorMode('invalid-mode'),error=>error.code==='GenericFailure');
  assert.equal(binding.derivePolicyContentClass(['Guide.MD','docs/spec.md']),'docs_only');
  assert.equal(binding.derivePolicyContentClass(['src/lib.rs']),'code_change');
  assert.equal(binding.derivePolicyAuthorClass('ai_only_experimental'),'ai_related');
  assert.equal(binding.derivePolicyAuthorClass(),null);
  const partial=binding.buildMinimumProvenance('ai_only_experimental');
  assert.equal(partial[0].evidence_readiness,'partial'); assert.equal(partial[0].policy_readable,false);
  const complete=binding.buildMinimumProvenance('ai_only_experimental','fixture-model');
  assert.equal(complete[0].evidence_readiness,'complete'); assert.equal(complete[1].minimum_evidence.model_name,'fixture-model');
  const policy=binding.normalizePolicy(binding.policyProfile('prototype'));
  assert.deepEqual(binding.parsePolicyYaml(binding.policyToYaml(policy)),policy);
  const effective=binding.resolveEffectivePolicy(policy,{contentClass:'docs_only',authorClass:'ai_related'});
  assert.deepEqual(effective.policy,policy); assert.equal(effective.content_class,'docs_only');
  const withOverride={...policy,class_overrides:[{when:{content_class:'docs_only',author_class:'ai_related'},set:{require_tests:false}}]};
  assert.equal(binding.resolveEffectivePolicy(withOverride,{contentClass:'docs_only',authorClass:'ai_related'}).effective_requirements.require_tests,false);
  assert.equal(binding.resolveEffectivePolicy(withOverride,{contentClass:'code_change',authorClass:'ai_related'}).effective_requirements.require_tests,true);
  assert.throws(() => binding.policyProfile('absent-profile'),error=>error.code==='GenericFailure');
  assert.throws(() => binding.parsePolicyYaml('line without a separator'),error=>error.code==='GenericFailure');
  const review='Reviewed files: lib.rs; Findings: none; Risks: low; Tests: unit checks; Recommendation: pass';
  assert.equal(binding.isStructuredCodeReviewSummary(review),true);
  assert.deepEqual(binding.missingCodeReviewSummarySections(review),[]);
  assert.ok(binding.missingCodeReviewSummarySections().length > 0);
  assert.match(binding.renderCodeReviewSummaryTemplate('numbered'),/^1\. Reviewed files/);
  assert.match(binding.codeReviewSummaryRequirementText(),/Reviewed files/);
  assert.throws(() => binding.renderCodeReviewSummaryTemplate('unknown'),error=>error.code==='GenericFailure');
  const explicit=path.join(os.tmpdir(),'ait-runtime-sdk-fixture');
  assert.equal(binding.resolveRuntimeDataRoot(explicit),explicit);
  assert.deepEqual(binding.resolveRuntimeDataRootWithSource(explicit),[explicit,'explicit']);
  assert.equal(binding.resolveServerRuntimeRoot(explicit),explicit);
  assert.deepEqual(binding.resolveServerRuntimeRootWithSource(explicit),[explicit,'explicit']);
  const envName=binding.runtimeDataEnvName(); const envValue=binding.runtimeDataEnvValue();
  assert.ok(envName===null || typeof envName==='string'); assert.ok(envValue===null || typeof envValue==='string');
  const ref='feature/SDK % 雪'; assert.equal(binding.decodeRefName(binding.encodeRefName(ref)),ref);
});

test('actual addon manages native source caches without a repository or CLI relay', () => {
  const fixture = fs.realpathSync(fs.mkdtempSync(path.join(os.tmpdir(),'ait-direct-source-cache-')));
  const core = path.join(fixture,'core'); const namespace = path.join(fixture,'cache');
  const write = (relative,body) => {const file=path.join(core,relative);fs.mkdirSync(path.dirname(file),{recursive:true});fs.writeFileSync(file,body);};
  try {
    write('rust/Cargo.toml','[workspace]\n');
    for (const crate of ['ait-cli','ait-core','ait-py','ait-server-core']) {
      write('rust/crates/'+crate+'/Cargo.toml','[package]\n');
      write('rust/crates/'+crate+'/src/lib.rs','pub fn fixture() {}\n');
    }
    const coreFingerprint = binding.currentCoreSourceFingerprint(core);
    assert.equal(binding.currentSourceCacheCoreFingerprint(core).fingerprint,coreFingerprint);
    assert.equal(binding.currentSourceCacheServerFingerprint(core).fingerprint,binding.currentServerSourceFingerprint(core));
    assert.ok(binding.currentSourceCacheCoreMtime(core).source_mtime_ns > 0n);
    assert.ok(binding.currentSourceCacheServerMtime(core).source_mtime_ns > 0n);
    const request={namespaceRoot:namespace,coreRepoRoot:core,extSuffix:'.fixture.so',coreSourceFingerprint:'core-a',workerId:'fixture'};
    const contract=binding.currentSourceNativeCacheContract(request);
    assert.equal(contract.worker_id,'fixture'); assert.match(binding.formatCurrentSourceCacheContract(contract),/^ait current-source-cache contract\n/);
    assert.notEqual(binding.currentSourceNativeCacheContract({...request,coreSourceFingerprint:'core-b'}).build_key,contract.build_key);
    const building=binding.currentSourceCacheMarkBuilding(request,18446744073709551615n,'{"fixture":true}');
    assert.equal(building.source_mtime_ns,18446744073709551615n); assert.equal(building.fixture,true);
    assert.match(binding.formatCurrentSourceCacheMarkBuilding(building),/^ait current-source-cache mark-building\n/);
    assert.throws(() => binding.currentSourceCacheMarkBuilding(request,1,'[]'),/JSON object/);
    assert.throws(() => binding.currentSourceCacheMarkBuilding(request,-1),/unsigned 64-bit/);
    assert.throws(() => binding.currentSourceCacheActivate(request,1,{ownerPid:4294967296n}),/unsigned 32-bit/);
    const payload=path.join(contract.cache_root,'runtime-extensions','ait_py','payload.bin');fs.mkdirSync(path.dirname(payload),{recursive:true});fs.writeFileSync(payload,'isolated cache fixture');
    const ready=binding.currentSourceCacheActivate(request,1n,{registerLease:true,ownerPid:process.pid});
    assert.equal(ready.manifest.state,'ready'); assert.ok(fs.existsSync(ready.lease.lease_path));
    assert.match(binding.formatCurrentSourceCacheActivate(ready),/^ait current-source-cache activate\n/);
    const artifact=path.join(fixture,'artifact.fixture');fs.writeFileSync(artifact,'cache freshness fixture');fs.chmodSync(artifact,0o755);
    const metadataPath=path.join(fixture,'metadata.json');
    fs.writeFileSync(metadataPath,binding.encodeJson({core_source_mtime_ns:1,core_source_fingerprint:'core-a',artifact_mtime:fs.statSync(artifact,{bigint:true}).mtimeNs,artifact_sha:createHash('sha256').update(fs.readFileSync(artifact)).digest('hex')}));
    const extension={metadataPath,extensionPath:artifact,sourceMtimeNs:1n,sourceFingerprint:'core-a'};
    assert.equal(binding.currentSourceCacheExtensionFresh(extension).fresh,true);
    assert.equal(binding.currentSourceCacheExtensionFresh({...extension,sourceFingerprint:'other'}).fresh,false);
    const binary={metadataPath,binaryPath:artifact,metadataFingerprintKey:'core_source_fingerprint',metadataSourceMtimeKey:'core_source_mtime_ns',metadataMtimeKey:'artifact_mtime',metadataShaKey:'artifact_sha',sourceMtimeNs:1n,sourceFingerprint:'core-a'};
    assert.equal(binding.currentSourceCacheBinaryFresh(binary).fresh,true);
    assert.equal(binding.currentSourceCacheBinaryFresh({...binary,sourceFingerprint:'other'}).fresh,false);
    assert.throws(() => binding.currentSourceCacheSeedCanonical({...request,coreSourceFingerprint:undefined},core,core,1),/requires --core-source-fingerprint/);
    binding.currentSourceCacheReleaseLease(ready.lease.lease_path,namespace,true);
    assert.equal(fs.existsSync(contract.cache_root),false);
    const pruned=binding.currentSourceCachePrune(namespace,{maxBytes:18446744073709551615n});
    assert.ok(Array.isArray(pruned.removed_idle));
    assert.equal(binding.formatCurrentSourceCachePrune(pruned),'ait current-source-cache prune');
  } finally { fs.rmSync(fixture,{recursive:true,force:true}); }
});

test('actual addon reports damaged content before returning the GC attention status', () => {
  const fixture = fs.realpathSync(fs.mkdtempSync(path.join(os.tmpdir(),'ait-direct-gc-')));
  const root = path.join(fixture,'repo'); fs.mkdirSync(root);
  try {
    binding.taskWorkflowInit(root);
    binding.configureRepository(root,{agent_teams:'disabled',user_name:'binding fixture',task_worktree_alias_root:path.join(fixture,'managed')});
    const task = binding.taskQuick(root,{intent:'Verify GC attention result',local:true});
    fs.writeFileSync(path.join(task.worktree.path,'content.txt'),'content in an isolated pack\n');
    binding.snapshotCapture(task.worktree.path,task.task_id,{message:'GC corruption fixture'});
    const packs = path.join(root,'.ait','objects','packs');
    const pack = fs.readdirSync(packs)[0]; assert.ok(pack);
    fs.writeFileSync(path.join(packs,pack),'invalid isolated fixture pack');
    const attention = binding.gcValidate(task.worktree.path);
    assert.equal(attention.state,'attention_required');
    assert.equal(attention.needs_attention,true);
    assert.ok(attention.issues.length > 0);
    assert.equal(binding.gcValidateExitCode(attention),1);
    assert.match(binding.formatGcValidate(attention),/state: attention_required/);
  } finally { fs.rmSync(fixture,{recursive:true,force:true}); }
});

test('actual addon imports, restores and exports an exact Git history through Rust', () => {
  const fixture = fs.realpathSync(fs.mkdtempSync(path.join(os.tmpdir(),'ait-direct-git-')));
  const root = path.join(fixture,'ait'); const source = path.join(fixture,'source');
  fs.mkdirSync(root); fs.mkdirSync(source);
  // Git commands build and inspect isolated interoperability fixtures only.
  const git = (cwd,...args) => {
    const result = spawnSync('git',args,{cwd,encoding:'utf8'});
    assert.equal(result.status,0,result.stderr); return result.stdout.trim();
  };
  try {
    git(source,'init','-b','main'); git(source,'config','user.name','Binding fixture');
    git(source,'config','user.email','fixture@example.invalid');
    const rawMarkdown = Buffer.from([101,120,97,99,116,13,10,255,0]);
    fs.writeFileSync(path.join(source,'README.md'),rawMarkdown);
    fs.writeFileSync(path.join(source,'code.txt'),'Git interop through Rust\n');
    git(source,'add','.'); git(source,'commit','-m','Fixture source');
    git(source,'tag','-a','v1','-m','Exact fixture tag');
    const commit = git(source,'rev-parse','HEAD');
    const refs = git(source,'show-ref');
    binding.taskWorkflowInit(root);
    binding.configureRepository(root,{agent_teams:'disabled',user_name:'binding fixture'});
    const migration = binding.gitMigrate(root,path.join(fixture,'migration'));
    assert.equal(migration.status,'not_required'); assert.equal(migration.mutated,false);
    assert.match(binding.formatGitMigrate(migration),/^ait git migrate\n/);
    assert.throws(() => binding.gitMigrate(root,path.join(root,'invalid-migration')),/isolated direct directory/);
    const preview = binding.gitImport(root,source,{allBranchesAndTags:true,dryRun:true});
    assert.equal(preview.mutated,false);
    assert.deepEqual(binding.taskWorkflowSnapshotList(root),[]);
    const imported = binding.gitImport(root,source,{allBranchesAndTags:true});
    assert.equal(imported.imported_commit_count,1);
    assert.ok(imported.generation_id); assert.match(binding.formatGitImport(imported),/^ait git import\n/);
    const restoredPath = path.join(fixture,'restored');
    const restored = binding.gitRestore(root,imported.generation_id,commit,restoredPath);
    assert.deepEqual(fs.readFileSync(path.join(restoredPath,'README.md')),rawMarkdown);
    assert.equal(fs.readFileSync(path.join(restoredPath,'code.txt'),'utf8'),'Git interop through Rust\n');
    assert.match(binding.formatGitRestore(restored),/^ait git restore\n/);
    const exportedPath = path.join(fixture,'export.git');
    const exported = binding.gitExport(root,exportedPath,{generation:imported.generation_id,allLinesAndTags:true});
    assert.equal(git(exportedPath,'show-ref'),refs);
    assert.equal(git(exportedPath,'rev-parse','HEAD'),commit);
    git(exportedPath,'fsck','--strict','--full');
    assert.match(binding.formatGitExport(exported),/^ait git export\n/);
    const mirrored = binding.gitMirror(root,exportedPath,'bidirectional',true);
    assert.equal(mirrored.mutated,false); assert.match(binding.formatGitMirror(mirrored),/^ait git mirror\n/);
    const recovered = binding.gitRecover(root);
    assert.equal(recovered.mutated,false); assert.match(binding.formatGitRecover(recovered),/^ait git recover\n/);
    assert.throws(() => binding.gitMirror(root,exportedPath,'invalid',true),/direction/);
    assert.throws(() => binding.gitImport(root,'-unsafe',{dryRun:true}),/must not begin/);
  } finally { fs.rmSync(fixture,{recursive:true,force:true}); }
});

test('actual addon preserves named remote Plan, repository and Patchset calls', async () => {
  const worker = new Worker(`
    const { parentPort } = require('node:worker_threads');
    const http = require('node:http');
    let calls = [];
    let reviews = [];
    let attestation = null;
    let remoteHead = null;
    const patchset = {task_id:'RT-1',change_id:null,change_ref:null,patchset_id:'RT-1/P-01',patchset_number:1,
      base_snapshot_id:'SNP-BASE',revision_snapshot_id:'SNP-REVISION',author_mode:'ai_only_experimental',publish_state:'published',evaluation_state:'pending',
      ci_run_seq:1,ci_completed_run_seq:1,ci:{run_seq:1,suite_result_count:1,blocking_failure_count:0,tests_status:'pass',lint_status:'pass'}};
    const server = http.createServer((req, res) => {
      calls.push(req.url);
      res.setHeader('content-type', 'application/json');
      if (req.url === '/v1/handshake') {
        res.end(JSON.stringify({operational_capabilities:{review_actor_identity_v1:true},ci_capabilities:{native_runner:{contract:'fixture.native-runner'},remote_sync_capabilities:{zstd_pack_bulk:true,zstd_pack_bulk_download:true}}}));
      } else if (req.url === '/v1/native/repository-authorities') {
        res.end(JSON.stringify({repository_index:7,repository_name:'binding fixture',namespace:'',actor_key_index:7}));
      } else if (req.url.startsWith('/v1/native/auth/whoami')) {
        res.end(JSON.stringify({identity:'reviewer@example.invalid',mode:'authenticated',repo_name:'binding fixture'}));
      } else if (req.url === '/v1/native/admin/repositories/binding%20fixture/bindings') {
        if (req.method === 'POST') {
          let body = ''; req.on('data', chunk => body += chunk);
          req.on('end', () => res.end(JSON.stringify({repo_name:'binding fixture',...JSON.parse(body)})));
        } else res.end(JSON.stringify([{actor_identity:'reviewer@example.invalid',role:'reviewer',created_at:'2026-10-26T00:00:00Z'}]));
      } else if (req.url === '/v1/native/repository-authorities/7') {
        res.end(JSON.stringify({repository:{repository_index:7,repository_name:'binding fixture',namespace:'',policy_flags:0,tombstoned:false,snapshot_capture_binding_contract:'ait.snapshot_capture_binding.v2'}}));
      } else if (req.url.startsWith('/v1/native/repository-authorities/7/snapshots/')) {
        res.writeHead(404);res.end(JSON.stringify({detail:'No durable capture in this read-only fixture'}));
      } else if (req.url === '/v1/native/repository-authorities/7/lines/main') {
        res.end(JSON.stringify({repo_name:'binding fixture',name:'main',line_name:'main',head_snapshot_id:remoteHead,status:'active'}));
      } else if (req.url.startsWith('/v1/native/repository-authorities/7/worker-jobs/')) {
        res.end(JSON.stringify({worker_job_index:77,job_type:'patchset.ci',state:'succeeded',state_kind:3}));
      } else if (req.url.startsWith('/v1/native/repository-authorities/7/worker-jobs?')) {
        res.end(JSON.stringify({jobs:[{worker_job_index:77,job_type:'patchset.ci',state:'succeeded',state_kind:3}],count:1}));
      } else if (req.url === '/v1/native/repository-authorities/7/tasks/RT-1') {
        res.end(JSON.stringify({task_id:'RT-1',status:'active',successful_land:null,current_patchset_id:patchset.patchset_id}));
      } else if (req.url === '/v1/native/repository-authorities/7/changes/RT-1/patchsets') {
        res.end(JSON.stringify([patchset]));
      } else if (req.url === '/v1/native/repository-authorities/7/changes/RT-1/reviews') {
        if (req.method === 'POST') {
          let body = ''; req.on('data', chunk => body += chunk);
          req.on('end', () => {const review={task_id:'RT-1',change_id:null,change_ref:null,...JSON.parse(body)};reviews.push(review);res.end(JSON.stringify(review));});
        } else res.end(JSON.stringify({task_id:'RT-1',change_id:null,change_ref:null,current_patchset_id:patchset.patchset_id,reviews,
          approvals:0,blocking:0,comments:0,task_approvals:reviews.filter(row=>row.action==='task_approve').length,
          code_review_summaries:reviews.filter(row=>row.action==='code_review_summary').length}));
      } else if (req.url.startsWith('/v1/native/repository-authorities/7/patchsets/RT-1%2FP-01')) {
        if (req.url.endsWith('/attestation')) {
          if (req.method === 'PUT') {
            let body='';req.on('data',chunk=>body+=chunk);req.on('end',()=>{attestation={...JSON.parse(body),task_id:'RT-1',change_id:null,change_ref:null,patchset_id:patchset.patchset_id,attestation_id:'AT-1',ci_backed:true};res.end(JSON.stringify(attestation));});
          } else if (attestation) res.end(JSON.stringify(attestation));
          else {res.writeHead(404);res.end(JSON.stringify({detail:'No attestation'}));}
        } else if (req.url.endsWith('/policy') || req.url.endsWith(':evaluatePolicy')) {
          res.end(JSON.stringify({policy_id:'PO-1',patchset_id:patchset.patchset_id,decision:attestation?'pass':'pending',input_fingerprint:'fixture-input',evaluated_at:'2026-10-26T00:00:00Z',checks:[{name:'require_tests',status:'pass',message:'Exact fixture CI'}]}));
        } else if (req.url.endsWith('/waivers')) {
          let body='';req.on('data',chunk=>body+=chunk);req.on('end',()=>res.end(JSON.stringify({...JSON.parse(body),waiver_id:'WV-1',patchset_id:patchset.patchset_id})));
        } else if (req.url.endsWith(':runCi')) res.end(JSON.stringify({patchset_id:patchset.patchset_id,queued:true,trigger:'manual_rerun'}));
        else res.end(JSON.stringify(patchset));
      } else if (req.url.startsWith('/v1/native/repository-authorities/7/read/patchsets/RT-1%2FP-01/ci-status')) {
        res.end(JSON.stringify({patchset_id:patchset.patchset_id,tests_status:'pass'}));
      } else if (req.url.includes('/sprints/')) {
        res.writeHead(400); res.end(JSON.stringify({detail:'fixture remote rejected the Plan selector'}));
      } else { res.end('[]'); }
    });
    server.listen(0, '127.0.0.1', () => parentPort.postMessage(server.address().port));
    parentPort.on('message', message => {
      if (message.head) { remoteHead = message.head; patchset.base_snapshot_id=message.head;patchset.revision_snapshot_id=message.head;parentPort.postMessage('head set'); }
      else if (message.patchsetTask) {patchset.task_id=message.patchsetTask;parentPort.postMessage('owner set');}
      else parentPort.postMessage(calls);
    });
  `, { eval: true });
  const fixture = fs.realpathSync(fs.mkdtempSync(path.join(os.tmpdir(), 'ait-direct-remote-')));
  const root = path.join(fixture, 'repo'); fs.mkdirSync(root);
  try {
    const [port] = await once(worker, 'message');
    binding.taskWorkflowInit(root);
    // Test fixture authority only; production frontends never construct storage facts.
    const configPath = path.join(root, '.ait', 'config.json');
    const config = JSON.parse(fs.readFileSync(configPath, 'utf8'));
    Object.assign(config, { repository_index: 7, default_remote: 'origin', remotes: {
      origin: { remote_id: 1, url: 'http://127.0.0.1:' + port, repo_name: 'binding fixture', created_at: '2026-10-25T00:00:00Z' },
    } });
    fs.writeFileSync(configPath, JSON.stringify(config));
    binding.configureRepository(root, {agent_teams:'disabled',user_name:'binding fixture',user_email:'fixture@example.invalid'});
    assert.deepEqual(binding.planList(root, { remoteName: 'origin' }), []);
    assert.throws(() => binding.planShow(root, 'PR-999', { remoteName: 'origin' }), error =>
      error.code === 'GenericFailure' && error.message.includes('fixture remote rejected'));
    const repository = binding.repositoryShow(root, 'origin');
    assert.equal(repository.repository.repository_index, 7);
    assert.match(binding.formatRepositoryShow(repository, 'origin'), /binding fixture/);
    const capabilities = binding.repositoryCiCapabilities(root, 'origin');
    assert.equal(capabilities.remote, 'origin');
    assert.match(binding.formatRepositoryCiCapabilities(capabilities, 'origin'), /origin/);
    const jobsOptions = {remoteName:'origin',state:'succeeded',limit:4};
    const jobs = binding.repositoryJobs(root, jobsOptions);
    assert.equal(jobs.jobs[0].worker_job_index, 77);
    assert.match(binding.formatRepositoryJobs(jobs, jobsOptions), /#77/);
    assert.equal(binding.repositoryJobs(root, {remoteName:'origin',workerJobIndex:77n}).worker_job_index, 77);
    assert.throws(() => binding.repositoryJobs(root, {remoteName:'origin',workerJobIndex:77,state:'failed'}), /cannot be combined/);
    assert.throws(() => binding.repositoryJobs(root, {remoteName:'origin',limit:0}), /limit/);
    assert.throws(() => binding.repositoryJobs(root, {remoteName:'origin',limit:1.5}), /unsigned 32-bit integer/);
    assert.throws(() => binding.repositoryJobs(root, {remoteName:'origin',workerJobIndex:4294967296n}), /unsigned 32-bit integer/);
    assert.throws(() => binding.repositoryRetire(root, 'absent', true), /absent/);
    assert.throws(() => binding.repositoryRestore(root, 'absent'), /absent/);
    assert.match(binding.formatRepositoryRetire({status:'aborted'}), /^repo retirement\nstatus: aborted$/);
    assert.match(binding.formatRepositoryRestore({status:'restored'}), /^repo restore\nstatus: restored$/);
    const actor = binding.taskWorkflowAuthWhoami(root, {remoteName:'origin'});
    assert.equal(actor.identity, 'reviewer@example.invalid');
    assert.match(binding.formatAuthWhoami(actor), /^ait auth whoami\n/);
    const roles = binding.taskWorkflowAuthBindings(root, {remoteName:'origin'});
    assert.equal(roles[0].role, 'reviewer');
    assert.match(binding.formatAuthBindings(roles), /reviewer@example.invalid/);
    const granted = binding.taskWorkflowAuthGrant(root, 'reviewer@example.invalid', ['reviewer'], {remoteName:'origin'});
    assert.deepEqual(granted.roles, ['reviewer']);
    assert.match(binding.formatAuthGrant(granted), /^ait auth grant\n/);
    assert.equal(binding.taskWorkflowPatchsetList(root, 'RT-1', {remoteName:'origin'})[0].patchset_id, 'RT-1/P-01');
    assert.equal(binding.patchsetList(root, 'RT-1', 'origin')[0].patchset_id, 'RT-1/P-01');
    assert.equal(binding.taskWorkflowPatchsetShow(root, 'RT-1/P-01', {remoteName:'origin'}).task_id, 'RT-1');
    assert.equal(binding.taskWorkflowPatchsetCiStatus(root, 'RT-1/P-01', {remoteName:'origin'}).tests_status, 'pass');
    assert.match(binding.formatPatchsetCiStatus({patchset_id:'RT-1/P-01',tests_status:'pending',status_notice:{message:'CI retry queued'},recommended_action:'retry'}), /CI retry queued/);
    assert.match(binding.formatPatchsetShow({patchset_id:'RT-1/P-01',summary:'prefix {"display_summary":"Readable summary"}'}), /Readable summary/);
    assert.equal(binding.taskWorkflowPatchsetRerunCi(root, 'RT-1/P-01', {remoteName:'origin'}).queued, true);
    const summary = 'Reviewed files: code.rs; Findings: none; Risks: low; Tests: direct API checks; Recommendation: pass';
    const codeReview = binding.reviewCodeSubmit(root, 'RT-1', 'RT-1/P-01', summary, {reviewer:'Einstein',remoteName:'origin'});
    assert.equal(codeReview.action, 'code_review_summary');
    assert.equal(codeReview.task_review.result.action, 'task_approve');
    assert.match(binding.formatReviewCodeSubmit(codeReview), /^ait review code submit\n/);
    const reviewSummary = binding.reviewShow(root, 'RT-1', 'origin');
    assert.ok(reviewSummary.reviews.some(row=>row.action==='code_review_summary' && row.reviewer_kind==='bot'));
    assert.equal(binding.projectReviewShow(reviewSummary).current_patchset_id, 'RT-1/P-01');
    assert.equal(binding.projectReviewShow(reviewSummary).reviews, undefined);
    assert.match(binding.formatReviewShow(reviewSummary), /code_review_summary/);
    const taskComment = binding.reviewTaskComment(root, 'RT-1', {patchsetId:'RT-1/P-01',message:'Task review note',remoteName:'origin'});
    assert.equal(taskComment.package_review, false);
    assert.equal(binding.projectReviewTaskOperation(taskComment).action, 'task_comment');
    assert.match(binding.formatReviewTaskComment(taskComment), /^ait review task comment\n/);
    assert.throws(() => binding.reviewTaskApprove(root, 'RT-1', 'RT-1/P-01', 'Manual approval', 'origin'), /configured as `automatic`/);
    assert.throws(() => binding.reviewTeamApprove(root, 'RT-1'), /only available when Agent Teams is enabled/);
    assert.throws(() => binding.reviewTaskApprove(root, 'RT-1', '@relative.json', 'message', 'origin'), /must be absolute/);
    const selector = '@' + path.join(root, 'not-created.json');
    assert.equal(binding.validateReviewPatchsetSelector(selector), selector);
    assert.equal(fs.existsSync(selector.slice(1)), false);
    assert.equal(binding.validateReviewPatchsetSelector(' RT-1/P-01 '), 'RT-1/P-01');
    assert.equal(binding.validatePublicPatchsetArgument(' RT-1/P-01 '), 'RT-1/P-01');
    assert.throws(() => binding.validateReviewPatchsetSelector('@relative.json'), /absolute-path/);
    assert.throws(() => binding.validateReviewPatchsetSelector(' '+selector), /surrounding whitespace/);
    assert.throws(() => binding.validateReviewPatchsetSelector('1'), /public Patchset reference/);
    assert.match(binding.formatReviewCodeTemplate(binding.taskWorkflowReviewCodeTemplate()), /Reviewed files/);
    const statement = binding.taskWorkflowAttestPut(root, {patchsetId:'RT-1/P-01',tests:'pass',authorMode:'ai_only_experimental',remoteName:'origin'});
    assert.equal(statement.verification_state, 'pass');
    assert.equal(statement.require_tests_pass, true);
    assert.match(binding.formatAttestPut(statement), /verification_state: pass/);
    assert.throws(() => binding.taskWorkflowAttestPut(root, {patchsetId:'RT-1/P-01',lint:'pass',remoteName:'origin'}), /no representation/);
    assert.match(binding.formatAttestShow(binding.taskWorkflowAttestShow(root,'RT-1/P-01',{remoteName:'origin'})), /AT-1/);
    const evaluated = binding.taskWorkflowPolicyEval(root,'RT-1/P-01',{remoteName:'origin'});
    assert.equal(evaluated.decision,'pass'); assert.match(binding.formatPolicyEval(evaluated), /^ait policy eval\n/);
    const policy = binding.taskWorkflowPolicyShow(root,'RT-1/P-01',{remoteName:'origin'});
    assert.equal(binding.projectPolicyShow(policy).input_fingerprint,undefined);
    assert.match(binding.formatPolicyShow(policy), /Exact fixture CI/);
    const waiver = binding.taskWorkflowPolicyWaive(root,'RT-1/P-01','fixture-rule','Fixture reason',{remoteName:'origin'});
    assert.equal(waiver.rule_name,'fixture-rule'); assert.match(binding.formatPolicyWaive(waiver), /WV-1/);
    assert.throws(() => binding.taskWorkflowPatchsetShow(root, '1', {remoteName:'origin'}), /Public Patchset reference/);
    const adapterOptions={repositoryIndex:7};
    const taskRemote=new binding.HttpTaskRemote(`http://127.0.0.1:${port}`,adapterOptions);
    assert.equal(taskRemote.getTask('RT-1').task_id,'RT-1');
    assert.deepEqual(taskRemote.listChanges('binding fixture'),[]);
    assert.equal(taskRemote.closeClient().closed,true);
    const closeout=new binding.HttpWorkflowCloseoutRemote(`http://127.0.0.1:${port}`,adapterOptions);
    assert.equal(closeout.listPatchsets('RT-1')[0].patchset_id,'RT-1/P-01');
    assert.equal(closeout.getPatchset('RT-1/P-01').task_id,'RT-1');
    assert.equal(closeout.readPatchsetCiStatus('RT-1/P-01',{recentLimit:3n,exactId:true}).tests_status,'pass');
    assert.equal(closeout.runPatchsetCi('RT-1/P-01',{exactId:true}).queued,true);
    assert.equal(closeout.listReviews('RT-1').task_id,'RT-1');
    assert.equal(closeout.getAttestation('RT-1/P-01',{exactId:true}).verification_state,'pass');
    assert.equal(closeout.getPolicy('RT-1/P-01',{exactId:true}).decision,'pass');
    assert.equal(closeout.evaluatePolicy('RT-1/P-01',{exactId:true}).decision,'pass');
    assert.equal(closeout.createWaiver({patchsetId:'RT-1/P-01',ruleName:'fixture-rule',reason:'Fixture reason',exactId:true}).waiver_id,'WV-1');
    assert.deepEqual(closeout.mutationReceipt({action:' record_review ',sourceAction:'workflow ready',delivery:'direct_response',responseRecovery:{state:'recovered',empty:null},result:{task_id:'RT-1',private:'discard'}}),{action:'record_review',source_action:'workflow ready',delivery:'direct_response',response_recovery:{state:'recovered'},task_id:'RT-1'});
    assert.equal(closeout.actionMutationReceipts('record_review',{review_id:'RV-1',policy_refresh:{decision:'pass'}}).length,2);
    assert.throws(()=>closeout.actionMutationReceipts('record_review',[]),/result must be an object/);
    assert.throws(()=>closeout.mutationReceipt({action:'',sourceAction:'workflow ready',delivery:'remote'}),/must be non-empty/);
    assert.throws(()=>closeout.putAttestation('RT-1/P-01','ai_only_experimental',{verification_state:'pass',extra:true}),/Unsupported compact Attestation/);
    assert.throws(()=>closeout.readPatchsetCiStatus('RT-1/P-01',{recentLimit:9223372036854775808n}),/must fit in i64/);
    assert.ok(closeout.inspectClient().request_count>=9);
    assert.equal(closeout.closeClient().closed,true);
    assert.throws(()=>closeout.getPatchset('RT-1/P-01'),/already closed/);
    binding.configureRepository(root, {task_worktree_alias_root:path.join(fixture,'managed')});
    const localTask = binding.taskQuick(root, {intent:'Verify remote synchronization',local:true});
    fs.writeFileSync(path.join(localTask.worktree.path,'sync.txt'),'durable local content\n');
    const localSnapshot = binding.snapshotCapture(localTask.worktree.path,localTask.task_id,{message:'Remote sync fixture'});
    assert.equal(binding.taskFinishExitCode(binding.taskFinish(root,localTask.task_id,{local:true})),0);
    worker.postMessage({head:localSnapshot.snapshot_id});
    assert.deepEqual(await once(worker,'message'),['head set']);
    const sourceBeforeBlame=fs.readFileSync(path.join(root,'sync.txt'));
    const taskBlame=binding.taskWorkflowBlame(root,'sync.txt',{patchsetId:'RT-1/P-01',remoteName:'origin',line:1n});
    assert.equal(taskBlame.target.task_id,'RT-1');
    assert.equal(taskBlame.target.change_id,null);assert.equal(taskBlame.target.change_ref,null);
    assert.equal(taskBlame.resolved_snapshot_id,localSnapshot.snapshot_id);assert.equal(taskBlame.lines.length,1);
    worker.postMessage({patchsetTask:'RT-2'});assert.deepEqual(await once(worker,'message'),['owner set']);
    assert.throws(()=>binding.taskWorkflowBlame(root,'sync.txt',{patchsetId:'RT-1/P-01',remoteName:'origin'}),error=>error.message==='Patchset reference disagrees with actual Task/optional Change');
    worker.postMessage({patchsetTask:'RT-1'});assert.deepEqual(await once(worker,'message'),['owner set']);
    assert.deepEqual(fs.readFileSync(path.join(root,'sync.txt')),sourceBeforeBlame);

    const pushed = binding.pushRepository(root,{remoteName:'origin',lineName:'main'});
    assert.equal(pushed.sync_reason,'remote_head_matches_local_head');
    assert.equal(pushed.uploaded_snapshots,0);
    assert.match(binding.formatPush(pushed),/^ait push\n/);
    const pulled = binding.pullRepository(root,{remoteName:'origin',lineName:'main'});
    assert.equal(pulled.relationship,'equal');
    assert.equal(pulled.imported_snapshots,0);
    assert.equal(pulled.head_snapshot_id,localSnapshot.snapshot_id);
    assert.match(binding.formatPull(pulled),/^ait pull\n/);
    assert.equal(binding.projectRemoteSync(pulled).phase_timings_ms,undefined);
    assert.equal(binding.projectRemoteSync(pulled).remote_sync_metrics,undefined);
    assert.equal(binding.taskWorkflowPush(root,{lineName:'main'}).sync_reason,pushed.sync_reason);
    assert.equal(binding.taskWorkflowPull(root,{lineName:'main'}).relationship,pulled.relationship);
    assert.throws(() => binding.pullRepository(root,{merge:true}),/--merge requires --restore/);
    assert.throws(() => binding.pullRepository(root,{force:true}),/--force only applies/);
    assert.throws(() => binding.taskWorkflowUploadSnapshotChain(root,localSnapshot.snapshot_id,{remoteName:'absent'}),/absent/);
    worker.postMessage('calls');
    const [calls] = await once(worker, 'message');
    assert.ok(calls.some(url => url.includes('/sprints')));
    assert.ok(calls.some(url => url.includes('/worker-jobs?') && url.includes('limit=4') && url.includes('state_kind=3')));
    assert.ok(calls.some(url => url.endsWith('/worker-jobs/77')));
    assert.ok(calls.some(url => url.endsWith('/patchsets/RT-1%2FP-01:runCi')));
    assert.ok(calls.some(url => url.startsWith('/v1/native/auth/whoami?')));
  } finally {
    await worker.terminate();
    fs.rmSync(fixture, { recursive: true, force: true });
  }
});

test('actual addon executes repository services and preserves native policy errors', () => {
  const root = fs.realpathSync(fs.mkdtempSync(path.join(os.tmpdir(), 'ait-direct-api-')));
  try {
    assert.equal(binding.taskWorkflowInit(root).action, 'initialized');
    binding.taskWorkflowConfigSet(root, { agent_teams: 'disabled', sprint: 'off', user_name: 'binding fixture' });
    assert.equal(binding.taskWorkflowConfigShow(root).agent_teams, false);
    assert.equal(binding.taskWorkflowStatus(root).workspace_root, root);
    assert.deepEqual(binding.taskWorkflowTaskList(root), []);
    assert.deepEqual(binding.taskWorkflowRemoteList(root), []);
    assert.deepEqual(binding.taskWorkflowSnapshotList(root), []);
    assert.deepEqual(binding.taskWorkflowChangeList(root, {local:true}), []);
    assert.equal(binding.taskWorkflowAuthWhoami(root).mode, 'open');
    assert.throws(() => binding.taskWorkflowAuthGrant(root, 'fixture', []), /At least one --role/);
    assert.throws(() => binding.taskWorkflowAuthBindings(root), error => error.code === 'GenericFailure');
    assert.throws(() => binding.taskWorkflowReviewTeamApprove(root, 'RT-1'), /only available when Agent Teams is enabled/);
    assert.throws(() => binding.taskWorkflowSnapshotCreate(root, 'blocked capture'), error =>
      error.code === 'GenericFailure' && error.message.includes('active Task worktree'));
    const configPath = path.join(root, '.ait', 'config.json');
    const original = fs.readFileSync(configPath);
    assert.throws(() => binding.taskWorkflowConfigSet(root, { workflow_mode: 'solo_local' }), error =>
      error.code === 'GenericFailure' && Boolean(error.message));
    assert.deepEqual(fs.readFileSync(configPath), original);
    assert.throws(() => binding.taskWorkflowConfigShow(123), error => error.code === 'StringExpected');
  } finally {
    fs.rmSync(root, { recursive: true, force: true });
  }
});

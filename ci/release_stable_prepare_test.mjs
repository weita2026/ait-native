#!/usr/bin/env node

import assert from "node:assert/strict";
import { spawn, spawnSync } from "node:child_process";
import { chmodSync, copyFileSync, existsSync, lstatSync, mkdtempSync, mkdirSync, readFileSync, readdirSync, rmSync, symlinkSync, writeFileSync } from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { createHash } from "node:crypto";
import {
  acceptedComponentCloseouts,
  closeoutReadinessProbes,
  bindingCard,
  copyOrVerify,
  materializeDirectory,
  replaceComponentSources,
  stableAdvance,
  writeOrVerify,
  admittedSourceRequest,
  inspectAdmittedOperation,
  inspectPreparedOperation,
  completedRemoteAdmission,
  qualifiedOperationInputs,
  materializeQualifiedSource,
} from "./release_stable_prepare.mjs";
import { validateWebOperation } from "./release_web_controls.mjs";

const here = path.dirname(fileURLToPath(import.meta.url));
const temporary = mkdtempSync(path.join(os.tmpdir(), "ait-release-prepare-test."));
const oldVersion = "1.1.2";
const version = "1.1.3";
const oldPin = "SNP-111111111111";
const pin = "SNP-222222222222";

function write(root, relative, text, mode = 0o644) {
  if (relative === "ait-release-family.json" && /^0:/.test(text)) {
    const count = text.trim().split("\n").length - 2;
    text = JSON.stringify({ schema: "ait.release.family/v3", family: { name: "ait-native", version: oldVersion, tag: `v${oldVersion}` },
      components: Array.from({ length: count }, (_, id) => ({ id: `fixture-${id}`, version: oldVersion })) });
  }
  const file = path.join(root, relative);
  mkdirSync(path.dirname(file), { recursive: true });
  writeFileSync(file, text, { mode });
}

function repeated(token, amount) {
  return Array.from({ length: amount }, (_, index) => `${index}:${token}`).join("\n") + "\n";
}

function packageLock(names) {
  return names.map((name) => `[[package]]\nname = "${name}"\nversion = "${oldVersion}"\n`).join("\n");
}

function external() {
  return `[[external]]\nname = "ait-core"\nsnapshot = "${oldPin}"\nversion = "${oldVersion}"\n`;
}

async function admittedBootstrap() {
  const root = path.join(temporary, "admitted"), native = path.join(root, "native");
  const ids = ["core", "server", "runner", "python", "node"], indices = [0, 1, 4, 2, 3];
  const roots = Object.fromEntries([...ids, "native"].map(id => [id, path.join(root, id)]));
  Object.values(roots).forEach(dir => mkdirSync(dir, { recursive: true }));
  const head = "SNP-999999999999", runnerPin = "SNP-AAAAAAAAAAAA";
  const sources = Object.fromEntries(ids.map((id, index) => [id, { task: `${index % 2 ? "L" : "R"}${id[0].toUpperCase()}T-${1000 + index}`,
    snapshot: `SNP-${String(index + 1).repeat(12)}`, edit_root: path.join(root, `cleaned-${id}`),
    stage: index % 2 ? "finished_local" : "snapshot_ready", repository_root: roots[id] }]));
  const declaration = (id, index, snapshot, materialize) => ({ name: `ait-${id}`, repo_name: `ait-${id}`, repository_index: index,
    remote: "origin", line: "main", snapshot, materialize_to: materialize,
    license: id === "server" ? "AGPL-3.0-only" : "Apache-2.0", version: "1.2.0" });
  const toml = row => Object.entries(row).map(([key, value]) => `${key} = ${JSON.stringify(value)}\n`).join("");
  const declarations = ids.map((id, index) => declaration(id, indices[index], sources[id].snapshot, `.ait-external/ait-${id}`));
  write(native, "ait-release-family.json", JSON.stringify({ family: { version: "1.2.0", tag: "v1.2.0" } }));
  write(native, "ait-external.toml", declarations.map(row => `[[external]]\n${toml(row)}`).join("\n"));
  write(native, "ait-external.lock", declarations.map(row => `[[node]]\nparent_path = ""\n${toml(row)}`).join("\n"));
  const runner = path.join(native, ".ait-external/ait-runner");
  const nestedRunner = { ...declaration("core", 0, runnerPin, ".ait-external/ait-core"), version: "1.1.4" };
  write(runner, "ait-external.toml", `[[external]]\n${toml(nestedRunner)}`);
  write(runner, "ait-external.lock", `[[node]]\nparent_path = ""\n${toml(nestedRunner)}`);
  // Retained393 external status has no remote, line or product version. Do
  // not manufacture declaration properties in a materialization response.
  const materialized = ({ name, repo_name, repository_index, snapshot, materialize_to, license }, parent_path = "", bindings = []) =>
    ({ name, repo_name, repository_index, snapshot, materialize_to, license, parent_path, bindings,
      state: "materialized", dirty: false, linked: false, link_path: null, outdated: false, lock_drift: false });
  const externals = declarations.map(row => materialized(row));
  for (const id of ["node", "python", "runner"]) {
    const parent = `.ait-external/ait-${id}`, snapshot = id === "runner" ? runnerPin : sources.core.snapshot;
    const materialize = `${parent}/.ait-external/ait-core`;
    const crate = { node: "ait-napi", python: "ait-py", runner: "ait-core" }[id];
    externals.push(materialized(declaration("core", 0, snapshot, materialize), parent,
      [{ language: "rust", kind: "cargo-path", path: `rust/crates/${crate}`, package: crate }]));
    write(native, `${materialize}/.ait-external-marker.json`, JSON.stringify({ format: "ait.external.materialized", repo_name: "ait-core",
      repository_index: 0, snapshot, parent_path: parent, materialize_to: materialize }));
  }
  const remoteTask = (task, snapshot, repo, index, change = null, number = 1) => ({ task_id: task, repo_name: repo, repository_index: index,
    status: "completed", workflow_state: "completed", current_patchset_id: `${task}/${change ? change + "/" : ""}P-${String(number).padStart(2, "0")}`, successful_land: {
      task_id: task, repo_name: repo, repository_index: index, status: "succeeded", governance_authority: true,
      target_line: "main", revision_snapshot_id: snapshot, landed_snapshot_id: snapshot,
      patchset_id: `${task}/${change ? change + "/" : ""}P-${String(number).padStart(2, "0")}`,
      change_id: change, change_ref: change ? `${task}/${change}` : null, submission_id: `${task}/L-01`, result: {
        selected_revision_snapshot_id: snapshot, landed_snapshot_id: snapshot, target_line: "main" } } });
  const patchset = task => ({ task_id: task.task_id, repo_name: task.repo_name, repo_id: String(task.repository_index),
    patchset_id: task.current_patchset_id, patchset_number: Number(task.current_patchset_id.slice(-2)),
    change_id: task.successful_land.change_id, change_ref: task.successful_land.change_ref,
    revision_snapshot_id: task.successful_land.revision_snapshot_id, governance_authority: true,
    invalidated: false, withdrawn: false, publish_state: "published", evaluation_state: "pass", ci: { overall_status: "pass" } });
  const localTask = (task, snapshot, repo, remote) => ({ task_id: task, repo_name: repo, status: "completed",
    local_finish: { task_id: task, status: "succeeded", target_line: "main", landed_snapshot_id: snapshot },
    publication_state: "published", published_remote_name: "origin", published_task_id: remote });
  const nativeLocal = localTask("LNAT-0032", head, "ait-native", "RNAT-0014");
  const data = { head, tasks: [nativeLocal], nativeLocal, nativeRemote: remoteTask("RNAT-0014", head, "ait-native", 28),
    externals, local: {}, remote: {}, indices: Object.fromEntries(ids.map((id, index) => [id, indices[index]])) };
  ids.forEach((id, index) => {
    const row = sources[id], remote = row.task.replace(/^L/, "R");
    if (index % 2) data.local[row.task] = localTask(row.task, row.snapshot, `ait-${id}`, remote);
    data.remote[remote] = remoteTask(remote, row.snapshot, `ait-${id}`, indices[index], id === "core" ? "C-01" : null, id === "core" ? 2 : 1);
  });
  data.patchsets = Object.fromEntries([data.nativeRemote, ...Object.values(data.remote)].map(task =>
    [`${task.task_id}/P-${task.current_patchset_id.slice(-2)}`, patchset(task)]));
  const read = (cwd, argv, missing) => {
    if (argv[0] === "status") return { ok: true, repo_name: "ait-native", line_name: "main", worktree: null,
      head_snapshot_id: data.head, workspace: { dirty: false, changed_count: 0 } };
    if (argv[0] === "external") return { command: "external status", externals: data.externals, lock_drifts: [] };
    if (argv[0] === "patchset") return data.patchsets[argv[2]];
    if (argv[1] === "list") return data.tasks;
    if (argv[2] === "LNAT-0032") return data.nativeLocal;
    if (argv[2] === "RNAT-0014") return data.nativeRemote;
    const row = argv.includes("--local") ? data.local[argv[2]] : data.remote[argv[2]];
    if (!row && !missing) throw new Error("fixture has no Task");
    return row ?? null;
  };
  assert(admittedSourceRequest({ adopt: sources }));
  assert(!admittedSourceRequest({ adopt: { ...sources, core: { ...sources.core, stage: "active" } } }));
  const proof = inspectAdmittedOperation(native, sources, read);
  assert.equal(externals.length, 8);
  assert(externals.every(row => !Object.hasOwn(row, "version") && !Object.hasOwn(row, "remote") && !Object.hasOwn(row, "line")));
  assert.deepEqual(proof.binding, { task: "LNAT-0032", snapshot: head, stage: "finished_local" });
  assert(!Object.hasOwn(proof.binding, "edit_root"));
  assert(proof.closeouts.every(row => row.task.startsWith("R") && row.patchset === `${row.task}/P-${row.id === "core" ? "02" : "01"}` && !row.finish_local_before_ready));
  const reject = change => {
    const saved = structuredClone(data);
    change(); assert.throws(() => inspectAdmittedOperation(native, sources, read));
    for (const key of Object.keys(data)) delete data[key]; Object.assign(data, saved);
  };
  reject(() => { data.tasks = []; });
  reject(() => { data.tasks.push(structuredClone(nativeLocal)); });
  reject(() => { data.head = "SNP-BBBBBBBBBBBB"; }); // Same bytes, another head.
  reject(() => { data.nativeLocal.published_task_id = "RNAT-0015"; });
  reject(() => { data.nativeRemote.successful_land.governance_authority = false; });
  reject(() => { data.remote.RCT_ignored = null; data.remote[sources.core.task].status = "active"; });
  reject(() => { data.remote[sources.core.task].successful_land.revision_snapshot_id = head; });
  reject(() => { data.remote[sources.core.task].current_patchset_id = `${sources.core.task}/P-02`; });
  const corePublic = `${sources.core.task}/P-02`;
  reject(() => { const task = data.remote[sources.core.task];
    task.current_patchset_id = task.successful_land.patchset_id = `${sources.core.task}/C-02/P-02`;
    task.successful_land.change_id = "C-02"; task.successful_land.change_ref = `${sources.core.task}/C-02`;
  });
  reject(() => { data.patchsets[corePublic].patchset_id = `${sources.core.task}/C-02/P-02`; });
  reject(() => { data.patchsets[corePublic].change_ref = `${sources.core.task}/C-02`; });
  reject(() => { data.patchsets[corePublic].change_id = "C-02"; });
  reject(() => { data.patchsets[corePublic].patchset_number = 3; });
  reject(() => { data.patchsets[corePublic].patchset_id = `${sources.core.task}/C-01/P-03`; data.patchsets[corePublic].patchset_number = 3; });
  reject(() => { data.patchsets[corePublic].revision_snapshot_id = head; });
  reject(() => { data.patchsets[corePublic].governance_authority = false; });
  reject(() => { data.patchsets[corePublic].ci.overall_status = "fail"; });
  reject(() => { delete data.patchsets[corePublic]; });
  reject(() => { data.local[sources.server.task].published_remote_name = "other"; });
  reject(() => { delete data.local[sources.server.task]; });
  reject(() => { data.externals[0].snapshot = head; });
  reject(() => { data.externals[0].dirty = true; });
  reject(() => { data.externals.splice(0, 1); });
  reject(() => { data.externals[0].state = "missing"; });
  reject(() => { data.externals[0].materialize_to = ".ait-external/wrong-core"; });
  reject(() => { data.externals[0].repo_name = "wrong-core"; });
  reject(() => { data.externals[0].repository_index = 28; });
  reject(() => { data.externals[0].license = "wrong-license"; });
  const rejectSource = change => {
    const files = ["ait-external.toml", "ait-external.lock", "ait-release-family.json"].map(file => [path.join(native, file), readFileSync(path.join(native, file))]);
    try { change(); assert.throws(() => inspectAdmittedOperation(native, sources, read)); }
    finally { for (const [file, bytes] of files) writeFileSync(file, bytes); }
  };
  for (const file of ["ait-external.toml", "ait-external.lock"]) rejectSource(() => {
    const target = path.join(native, file); writeFileSync(target, readFileSync(target, "utf8").replace('version = "1.2.0"', 'version = "1.1.4"'));
  });
  rejectSource(() => { // Even matching declarations must agree with the family.
    for (const file of ["ait-external.toml", "ait-external.lock"]) {
      const target = path.join(native, file); writeFileSync(target, readFileSync(target, "utf8").replaceAll('version = "1.2.0"', 'version = "1.1.4"'));
    }
  });
  rejectSource(() => { const file = path.join(native, "ait-release-family.json"), family = JSON.parse(readFileSync(file));
    family.family = { version: "1.1.4", tag: "v1.1.4" }; writeFileSync(file, JSON.stringify(family));
  });
  const wrong = structuredClone(sources); wrong.core.snapshot = head;
  assert.throws(() => inspectAdmittedOperation(native, wrong, read));
  assert.throws(() => inspectAdmittedOperation(native, sources, read, { ...proof.binding, task: "LNAT-0031" }));
  assert.throws(() => inspectAdmittedOperation(native, sources, read, null, "1.1.3"));
  assert.throws(() => completedRemoteAdmission(data.nativeRemote, "RNAT-0014", head, "ait-core", 0));

  // Exercise the real request -> prepare -> accepted input -> spec -> plan ->
  // conductor -> Web input path. Only external authorities and other phase
  // commands are fixtures; no state or completed action is handwritten.
  const dataPath = path.join(root, "readbacks.json"), calls = path.join(root, "calls.jsonl");
  const ait = path.join(root, "ait"), home = path.join(root, "home");
  const persist = () => writeFileSync(dataPath, JSON.stringify(data)); persist();
  writeFileSync(ait, `#!${process.execPath}\nimport fs from 'node:fs'; import path from 'node:path';
const d=JSON.parse(fs.readFileSync(${JSON.stringify(dataPath)})), a=process.argv.slice(2), id=path.basename(process.cwd());
fs.appendFileSync(${JSON.stringify(calls)},JSON.stringify({id,a})+'\\n'); let r;
if(id==='web') {
const status={ok:true,repo_name:'ait-web-test',line_name:'main',head_snapshot_id:'SNP-CCCCCCCCCCCC',worktree:null,workspace:{dirty:false,changed_count:0},
current_line:'main',workspace_dirty:false,worktree_name:null,worktree_hygiene:{manual_review_candidate_count:0,stale_count:0},reconciliation:{total_finding_count:0}};
const config={repo_name:'ait-web-test',repository_index:23,id_namespace_prefix:{value:'WT'},workflow_default_scope:{workflow:{value:'local',source:'fixture'},task:{value:'remote',source:'fixture'},change:{value:'local',source:'fixture'}},agent_teams:true,sprint:{value:'on',enabled:true,source:'fixture'},default_author_mode:'ai_with_human_review',default_model:null,task_review:{value:'automatic',source:'fixture'}};
r=({status,config,queue:{summary:{attention_required_count:0,dirty_worktree_count:0},remote:{reviewer_inbox:{count:0}}},worktree:[],repo:{repository:{repository_index:23,repository_name:'ait-web-test',namespace:'WT',tombstoned:false}}})[a[0]];
} else if(a[0]==='status') r={ok:true,repo_name:id==='community'?'ait-web-community':'ait-native',line_name:'main',head_snapshot_id:id==='community'?'SNP-DDDDDDDDDDDD':d.head,worktree:null,workspace:{dirty:false,changed_count:0}};
else if(a[0]==='config') r={repo_name:'ait-'+id,repository_index:d.indices[id]};
else if(a[0]==='external'&&a[1]==='status') r={command:'external status',externals:d.externals,lock_drifts:[]};
else if(a[0]==='patchset'&&a[1]==='show') r=d.patchsets[a[2]];
else if(a[0]==='task'&&a[1]==='list') r=d.tasks;
else if(a[0]==='task'&&a[1]==='show') {r=a[2]==='LNAT-0032'?d.nativeLocal:a[2]==='RNAT-0014'?d.nativeRemote:(a.includes('--local')?d.local:d.remote)[a[2]];
if(!r&&a.includes('--local')){process.stderr.write('Error: Unknown Task: '+a[2]+'\\n');process.exit(1);}}
if(r===undefined) {process.stderr.write('unexpected native invocation '+a.join(' '));process.exit(99);}console.log(JSON.stringify(r));\n`, { mode: 0o755 });
  // Keep the actual Node runtime beside fixture gh, so releasePath selects
  // fixture credentials on hosts that install both node and gh in /usr/bin.
  write(home, ".cargo/bin/gh", `#!${process.execPath}\nimport fs from 'node:fs'; const a=process.argv.slice(2), head='a'.repeat(40);
if(a[0]==='auth'&&a[1]==='status') process.exit(0);
if(a[0]==='run'&&a[1]==='list') {const workflow=a[a.indexOf('--workflow')+1];console.log(JSON.stringify(fs.existsSync(${JSON.stringify(root + "/gh-")}+workflow)?[{databaseId:1000,headSha:head,status:'completed',conclusion:'success'}]:[]));}
else if(a[0]==='run'&&a[1]==='view') console.log(JSON.stringify({status:'completed',conclusion:'success',headSha:head,attempt:1,url:'https://example.test/run/1000'}));
else process.exit(99);\n`, 0o755);
  const childNode = path.join(home, ".cargo/bin/node"); copyFileSync(process.execPath, childNode); chmodSync(childNode, 0o755);
  const nodeLibraries = path.resolve(path.dirname(process.execPath), "../lib");
  if (process.platform === "darwin" && existsSync(nodeLibraries)) symlinkSync(nodeLibraries, path.join(home, ".cargo/lib"), "dir");
  const service = path.join(root, "service"); writeFileSync(service, "#!/bin/sh\nexit 99\n", { mode: 0o755 });
  const server = spawn(process.execPath, ["--input-type=module", "-e", `import http from 'node:http'; const s=http.createServer((q,r)=>{r.setHeader('Content-Type','application/json');r.end(JSON.stringify({repository:{repository_name:'ait-core'}}));});s.listen(0,'127.0.0.1',()=>console.log(s.address().port));`], { stdio: ["ignore", "pipe", "inherit"] });
  try {
    const port = await new Promise((resolve, reject) => { server.stdout.once("data", bytes => resolve(Number(String(bytes).trim()))); server.once("error", reject); });
    write(roots.core, ".ait/config.json", JSON.stringify({ default_remote: "origin", remotes: { origin: { url: `http://127.0.0.1:${port}` } } }));
    const webRoot = path.join(root, "web"); mkdirSync(webRoot);
    write(webRoot, "acceptance/manifests/ait-command-matrix.tsv", "fixture\n");
    const records = path.join(root, "records"), requestPath = path.join(root, "request.json");
    const request = { contract: "ait.release.request/v1", release: { channel: "stable", prior_version: "1.1.4", version: "1.2.0" },
      records_root: records, roots, paths: { public_source: root, web_test_root: webRoot },
      web: { server_url: "http://127.0.0.1:8088/", server_data: root, seed: 42, acceptance_snapshot: "SNP-CCCCCCCCCCCC" },
      github_repository: "owner/repository", winget_fork: "owner/winget-pkgs", adopt: Object.fromEntries(ids.map(id => {
        const { repository_root, ...row } = sources[id]; return [id, row];
      })), server_refresh: { stop_argv: [service], start_argv: [service], health_url: "http://127.0.0.1:8088/healthz" } };
    writeFileSync(requestPath, JSON.stringify(request));
    const env = { ...process.env, HOME: home, PATH: `${home}/.cargo/bin:${process.env.PATH}`, AIT_RELEASE_AIT_BIN: ait, AIT_RELEASE_OPERATION_ROOT: "", AIT_RELEASE_OPERATION_TOKEN: "" };
    const run = (script, args, status = 0) => {
      const result = spawnSync(childNode, [path.join(here, script), ...args], { encoding: "utf8", env });
      const log = /see (.+\.log)/.exec(result.stderr ?? "")?.[1];
      assert.equal(result.status, status, `${script}: ${result.stdout}\n${result.stderr}\n${log && existsSync(log) ? readFileSync(log, "utf8") : ""}`); return result;
    };
    const omittedRefresh = structuredClone(request); delete omittedRefresh.server_refresh;
    for (const mode of ["plan", "run"]) {
      for (const refresh of [null, {}, { stop_argv: [], start_argv: [service], health_url: request.server_refresh.health_url }]) {
        writeFileSync(requestPath, JSON.stringify({ ...omittedRefresh, server_refresh: refresh }));
        assert.match(run("release_stable_prepare.mjs", [mode, "--request", requestPath], 65).stderr, /server_refresh/);
      }
      const notAdmitted = structuredClone(omittedRefresh); notAdmitted.adopt.core.stage = "active";
      writeFileSync(requestPath, JSON.stringify(notAdmitted));
      assert.match(run("release_stable_prepare.mjs", [mode, "--request", requestPath], 65).stderr, /server_refresh/);
      notAdmitted.server_refresh = { ...request.server_refresh, stop_argv: [path.join(root, "missing-stop")] };
      writeFileSync(requestPath, JSON.stringify(notAdmitted));
      const rejected = spawnSync(childNode, [path.join(here, "release_stable_prepare.mjs"), mode, "--request", requestPath], { encoding: "utf8", env });
      assert.notEqual(rejected.status, 0); assert.match(rejected.stderr, /server_refresh.stop_argv/);
    }
    // Unused lifecycle executable paths are not preflight requirements for
    // this route. A supplied object still passed the same format validation.
    writeFileSync(requestPath, JSON.stringify({ ...omittedRefresh,
      server_refresh: { ...request.server_refresh, stop_argv: [path.join(root, "unused-stop")], start_argv: [path.join(root, "unused-start")] } }));
    run("release_stable_prepare.mjs", ["plan", "--request", requestPath]);
    writeFileSync(requestPath, JSON.stringify(omittedRefresh));
    run("release_stable_prepare.mjs", ["plan", "--request", requestPath]);
    assert(!existsSync(path.join(records, "preparation-state.json")));
    run("release_stable_prepare.mjs", ["run", "--request", requestPath]);
    const preparationPath = path.join(records, "preparation-state.json"), acceptedPath = path.join(records, "accepted-input.json");
    const preparation = JSON.parse(readFileSync(preparationPath)), accepted = JSON.parse(readFileSync(acceptedPath));
    assert.deepEqual(preparation.binding, proof.binding);
    assert.deepEqual(accepted.component_closeouts, proof.closeouts);
    for (const step of ["build_server", "refresh_server", "refresh_core_artifacts", "publish_core_source", "publish_node_source"]) assert(!preparation.steps[step]);
    const map = JSON.parse(readFileSync(accepted.paths.web_components));
    assert.deepEqual(map, { ...proof.components, web_test: request.web.acceptance_snapshot });
    inspectPreparedOperation(preparationPath, acceptedPath, read);
    run("release_stable_prepare.mjs", ["run", "--request", requestPath]);
    // The ordinary preparation producer carries the Community source into
    // accepted input and the spec; an independent source cannot be substituted.
    const communityRoot = path.join(root, "community"); mkdirSync(communityRoot);
    const fullRequest = structuredClone(omittedRefresh); fullRequest.records_root = path.join(root, "community-records");
    Object.assign(fullRequest.paths, { community_root: communityRoot, community_bin: service, community_cli_bin: service, chrome: service });
    fullRequest.web.community_snapshot = "SNP-DDDDDDDDDDDD";
    const fullRequestPath = path.join(root, "community-request.json"); writeFileSync(fullRequestPath, JSON.stringify(fullRequest));
    run("release_stable_prepare.mjs", ["run", "--request", fullRequestPath]);
    const fullAcceptedPath = path.join(fullRequest.records_root, "accepted-input.json"), fullPreparationPath = path.join(fullRequest.records_root, "preparation-state.json");
    const fullAccepted = JSON.parse(readFileSync(fullAcceptedPath)), fullMapBytes = readFileSync(fullAccepted.paths.web_components);
    assert.equal(fullAccepted.paths.community_root, communityRoot);
    assert.equal(JSON.parse(fullMapBytes).community, fullRequest.web.community_snapshot);
    inspectPreparedOperation(fullPreparationPath, fullAcceptedPath, read);
    writeFileSync(fullAccepted.paths.web_components, JSON.stringify({ ...JSON.parse(fullMapBytes), community: head }));
    assert.throws(() => inspectPreparedOperation(fullPreparationPath, fullAcceptedPath, read), /prepared Web source identities differ/);
    writeFileSync(fullAccepted.paths.web_components, fullMapBytes);
    const fullSpec = path.join(fullRequest.records_root, "release-spec.json");
    run("release_stable_spec.mjs", ["--input", fullAcceptedPath, "--output", fullSpec]);
    assert.equal(JSON.parse(readFileSync(fullSpec)).values.community_root, communityRoot);
    data.head = "SNP-BBBBBBBBBBBB"; persist();
    run("release_stable_prepare.mjs", ["run", "--request", requestPath], 75);
    data.head = head; persist();

    const specPath = path.join(records, "release-spec.json"), recipePath = path.join(root, "recipe.json");
    run("release_stable_spec.mjs", ["--input", acceptedPath, "--output", specPath]);
    const recipe = JSON.parse(readFileSync(path.join(here, "../release/stable-conductor.recipe.json")));
    for (const phase of recipe.phases) if (!["component_release", "web_admission"].includes(phase.id)) {
      const output = path.join(records, `${phase.id}.json`);
      phase.actions = [{ id: `${phase.id}-fixture`, type: "command", cwd: root, mutation: false,
        argv: [process.execPath, "-e", "require('fs').writeFileSync(process.argv[1],'{}')", output], outputs: [output] }];
      if (["public_qualification", "component_receipts", "clean_host_qualification", "protected_promotion", "endpoint_publication", "latest_alias"].includes(phase.id)) {
        phase.actions = [{ id: `${phase.id}-fixture`, type: "github_workflow", cwd: root, repository: "owner/repository", workflow: phase.id,
          head_sha: "a".repeat(40), dispatch_argv: [process.execPath, "-e", `require('fs').writeFileSync(${JSON.stringify(root + "/gh-" + phase.id)},'dispatched')`] }];
      }
      if (["tag", "winget_submission"].includes(phase.id)) Object.assign(phase.actions[0], { mutation: true,
        probe: { cwd: root, argv: [process.execPath, "-e", "process.exit(0)"] } });
    }
    const candidatePath = path.join(records, "candidate.json");
    recipe.phases.find(row => row.id === "candidate_freeze").actions[0] = { id: "candidate-fixture", type: "command", cwd: root, mutation: false,
      argv: [process.execPath, "-e", `require('fs').writeFileSync(process.argv[1],JSON.stringify(${JSON.stringify({ contract: "ait.release.operator.pre-tag-candidate-binding/v1", status: "ready_for_immutable_tag",
        release: { id: "REL-FAM-1111111111111111", ...request.release },
        candidate_authority: { sha256: "a".repeat(64), record: { prior_release: { version: request.release.prior_version, python_version: request.release.prior_version } } } })}))`, candidatePath], outputs: [candidatePath] };
    const webPhase = recipe.phases.find(row => row.id === "web_admission");
    const webInput = path.join(records, "web-admission/qualification-input.json");
    const bind = webPhase.actions[0]; bind.argv[1] = path.join(here, "release_web_admission_init.mjs");
    bind.argv[bind.argv.indexOf("--ait") + 1] = ait;
    bind.argv[bind.argv.indexOf("--server-bin") + 1] = service;
    // Retain the actual bind action and stop at the following evidence gate.
    webPhase.actions = [bind, { id: "await-web-fixture", type: "evidence_gate", path: path.join(records, "web-pass"), message: "fixture Web gate" }];
    writeFileSync(recipePath, JSON.stringify(recipe));
    const planPath = path.join(records, "conductor-plan.json"), statePath = path.join(records, "conductor-state.json");
    run("release_conductor_init.mjs", ["--spec", specPath, "--recipe", recipePath, "--plan", planPath]);
    const plan = JSON.parse(readFileSync(planPath));
    assert.doesNotMatch(readFileSync(planPath, "utf8"), /[A-Z]+T-[0-9]{4}\/C-[0-9]{2}/);
    assert.equal(plan.phases[1].actions.find(row => row.id === "closeout-core").patchset, corePublic);
    assert.equal(plan.phases.length, 14); assert(plan.source.control_files[preparationPath]);
    assert(!existsSync(statePath));
    const conductor = ["run", "--plan", planPath, "--state", statePath];
    run("release_conductor.mjs", conductor, 75);
    const state = JSON.parse(readFileSync(statePath));
    const action = state.phases.find(row => row.id === "web_admission").actions[0];
    assert.equal(action.id, "bind-web-admission-input"); assert.equal(action.status, "complete");
    assert.match(action.outputs[webInput], /^[0-9a-f]{64}$/);
    const bound = JSON.parse(readFileSync(webInput)); validateWebOperation(webInput, bound, webRoot);
    assert(!Object.hasOwn(bound.executables, "community")); assert.deepEqual(bound.browser_evidence, {});
    assert.equal(Object.keys(action.outputs).length, 7);
    for (const file of Object.keys(action.outputs)) assert(existsSync(file));
    // Missing action, incomplete status and changed digest remain HOLDs on a
    // genuinely initialized operation, independently of the protocol fixtures.
    const stateBytes = readFileSync(statePath);
    for (const change of [value => { value.phases.find(row => row.id === "web_admission").actions = []; },
      value => { value.phases.find(row => row.id === "web_admission").actions[0].status = "pending"; },
      value => { value.phases.find(row => row.id === "web_admission").actions[0].outputs[webInput] = "0".repeat(64); }]) {
      const changed = JSON.parse(stateBytes); change(changed); writeFileSync(statePath, JSON.stringify(changed));
      assert.throws(() => validateWebOperation(webInput, bound, webRoot));
    }
    writeFileSync(statePath, stateBytes);
    for (const row of state.phases.find(row => row.id === "component_release").actions) {
      assert.equal(row.status, "complete"); assert(!row.completion); assert(!row.expected_identity);
      assert.equal(Object.keys(row.outputs).length, 2);
    }
    const originalReceipt = readFileSync(Object.keys(state.phases[1].actions[0].outputs)[0]);
    run("release_conductor.mjs", conductor, 75); // Resume observes, never re-applies.
    assert.deepEqual(readFileSync(Object.keys(state.phases[1].actions[0].outputs)[0]), originalReceipt);
    const coreState = state.phases[1].actions.find(row => row.id === "closeout-core");
    const formalBytes = Object.keys(coreState.outputs).map(file => [file, readFileSync(file)]);
    // A different canonical Change or Submission cannot replace a frozen
    // admission, even when public Task/P and exact source bytes still match.
    for (const mutate of [() => {
      const task = data.remote[sources.core.task];
      task.current_patchset_id = task.successful_land.patchset_id = `${sources.core.task}/C-02/P-02`;
      task.successful_land.change_id = "C-02"; task.successful_land.change_ref = `${sources.core.task}/C-02`;
      data.patchsets[corePublic] = patchset(task);
    }, () => { data.remote[sources.core.task].successful_land.submission_id = `${sources.core.task}/L-02`; }]) {
      const saved = structuredClone(data); mutate(); persist();
      assert.match(run("release_conductor.mjs", conductor, 75).stderr, /recorded formal Change\/Patchset\/Submission identity drifted/);
      Object.assign(data, saved); persist();
      run("release_conductor.mjs", conductor, 75);
      for (const [file, bytes] of formalBytes) assert.deepEqual(readFileSync(file), bytes);
      assert.equal(JSON.parse(readFileSync(statePath)).phases[1].actions.find(row => row.id === "closeout-core").status, "complete");
    }
    const frozenConfig = Object.keys(action.outputs).find(file => file.endsWith("starting-config.json"));
    const configBytes = readFileSync(frozenConfig); writeFileSync(frozenConfig, `${configBytes}\n`);
    assert.match(run("release_conductor.mjs", conductor, 65).stderr, /completed release receipt drifted/);
    writeFileSync(frozenConfig, configBytes);
    for (const mutate of [() => { data.head = "SNP-BBBBBBBBBBBB"; },
      () => { data.remote[sources.core.task].successful_land.governance_authority = false; },
      () => { data.externals[0].snapshot = head; }]) {
      const saved = structuredClone(data); mutate(); persist(); run("release_conductor.mjs", conductor, 75);
      Object.assign(data, saved); persist();
    }
    const savedPreparation = readFileSync(preparationPath);
    writeFileSync(preparationPath, `${savedPreparation}\n`);
    assert.match(run("release_conductor.mjs", conductor, 65).stderr, /control source drifted/);
    writeFileSync(preparationPath, savedPreparation);
    const savedManifest = readFileSync(path.join(native, "ait-external.toml"));
    writeFileSync(path.join(native, "ait-external.toml"), `${savedManifest}\n`);
    assert.match(run("release_conductor.mjs", conductor, 65).stderr, /control source drifted/);
    writeFileSync(path.join(native, "ait-external.toml"), savedManifest);
    // The qualified operation is immutable input data. Its stopped state is
    // deliberately unusable; the new operation must execute native producers.
    const qualifiedRoot = path.join(root, "qualified"), fresh = path.join(root, "fresh");
    const publicRoot = path.join(root, "public"); mkdirSync(publicRoot);
    const git = (...args) => { const result = spawnSync("git", ["-C", publicRoot, ...args], { encoding: "utf8" });
      assert.equal(result.status, 0, result.stderr); return result.stdout.trim(); };
    git("init", "-b", "main"); git("config", "user.name", "Release fixture"); git("config", "user.email", "fixture@example.test");
    write(publicRoot, "source.txt", "1.1.4\n");
    write(publicRoot, "ait-release-family.json", JSON.stringify({ public_source: { identity: "weita2026/ait-native" } }));
    write(publicRoot, "build-release.mjs", `import assert from 'node:assert/strict';import{execFileSync}from'node:child_process';import{fileURLToPath}from'node:url';
assert.deepEqual(process.argv.slice(2),['--validate-only','--git-commit',execFileSync('git',['-C',fileURLToPath(new URL('.',import.meta.url)),'rev-parse','HEAD'],{encoding:'utf8'}).trim()]);\n`);
    git("add", "."); git("commit", "-m", "qualified source");
    const q = git("rev-parse", "HEAD");
    write(publicRoot, "source.txt", "1.2.0\n"); git("add", "."); git("commit", "-m", "version delta");
    const f = git("rev-parse", "HEAD"), hash = file => createHash("sha256").update(readFileSync(file)).digest("hex");
    const json = (name, value) => write(qualifiedRoot, name, `${JSON.stringify(value, null, 2)}\n`);
    const release = { ...request.release, source_commit: f, id: "REL-FAM-1111111111111111" };
    const mutation = { artifact_rebuild: false, tag_write: false };
    json("web-components.json", map);
    json("provider-run.json", { id: 101, run_attempt: 1, name: `AIT pre-RC qualification / ${q}`, display_title: `AIT pre-RC qualification / ${q}`,
      path: ".github/workflows/ait-release-pre-rc-qualification.yml", event: "workflow_dispatch", status: "completed", conclusion: "success", head_sha: q });
    json("provider-artifact.json", { id: 190, name: `ait-pre-rc-qualification-${q}`, digest: `sha256:${"9".repeat(64)}`, expired: false, workflow_run: { id: 101 } });
    json("provider-evidence/ait-release.pre-rc-qualification.json", { contract: "ait.release.pre-rc-qualification/v1", status: "qualified",
      source: { repository: "weita2026/ait-native", git_commit: q, workflow_control_commit: q, release_tags_at_qualification: [] },
      gates: { core_quality: "pass", release_controls: "pass", command_inventory: "pass", environment_inventory: "pass",
        windows: Object.fromEntries(["aarch64-pc-windows-msvc", "x86_64-pc-windows-msvc"].map(target => [target, { core_init: "pass", server_init: "pass", server_run: "pass" }])) },
      immutable_release_tag_created: false, release_receipts_created: false, public_endpoint_writes: false });
    const realOperator = path.join(here, "release_operator.sh");
    const providerArgs = ["--run-record", path.join(qualifiedRoot, "provider-run.json"), "--artifact-record", path.join(qualifiedRoot, "provider-artifact.json"),
      "--evidence-root", path.join(qualifiedRoot, "provider-evidence")];
    const realQualification = (source, output) => spawnSync(realOperator, ["bind-qualification", "--source-root", source, ...providerArgs, "--output", output], { encoding: "utf8" });
    const wrongQ = realQualification(publicRoot, path.join(qualifiedRoot, "wrong-final.json"));
    assert.equal(wrongQ.status, 65); assert(wrongQ.stderr.includes(`workflow run 101 is not the exact successful AIT pre-RC qualification / ${f} run`));
    const originalQ = { qualificationRoot: path.join(qualifiedRoot, "original-q-source"), qualification: { source: { git_commit: q } } };
    materializeQualifiedSource({ paths: { public_source: publicRoot } }, originalQ);
    const boundQ = realQualification(originalQ.qualificationRoot, path.join(qualifiedRoot, "pre-rc-qualification.json"));
    assert.equal(boundQ.status, 0, boundQ.stderr);
    materializeQualifiedSource({ paths: { public_source: publicRoot } }, originalQ); // The real consumer left Q clean.
    assert.equal(git("rev-parse", "HEAD"), f); assert.equal(git("status", "--porcelain", "--untracked-files=all"), "");
    json("pre-tag-admission.json", { contract: "ait.release.operator.pre-tag-admission/v1", status: "ready_for_component_receipts",
      qualification: { binding_sha256: hash(path.join(qualifiedRoot, "pre-rc-qualification.json")), source_commit: q }, release, mutation });
    const preRc = { admission_sha256: hash(path.join(qualifiedRoot, "pre-tag-admission.json")), qualification_workflow_run_id: 101, qualified_source_commit: q };
    json("prepare.json", { contract: "ait.release.operator.prepare/v3", status: "ready_for_component_receipts", pre_rc_admission: preRc, mutation });
    const authority = { sha256: "a".repeat(64), record: { prior_release: { version: request.release.prior_version } } };
    const dossier = { workflow_run_id: 102 };
    json("receipts.json", { contract: "ait.release.operator.receipt-binding/v2", status: "ready_for_pre_tag_qualification", release,
      pre_rc_admission: preRc, candidate_authority: authority, source_dossier: dossier, mutation });
    const zip = Buffer.from("provider candidate fixture"), zipHash = createHash("sha256").update(zip).digest("hex");
    json("candidate.json", { contract: "ait.release.operator.pre-tag-candidate-binding/v1", status: "ready_for_immutable_tag", release,
      candidate_authority: authority, source_dossier: dossier, mutation,
      qualification: { workflow_run_id: 103, workflow_control_commit: f, clean_host_rows: 24, tag_state_at_closeout: "absent",
        candidate_artifact_id: 104, candidate_artifact_digest: `sha256:${zipHash}` } });
    const selectedRequest = structuredClone(request); selectedRequest.records_root = fresh;
    selectedRequest.paths.public_source = publicRoot; selectedRequest.paths.web_components = path.join(qualifiedRoot, "web-components.json");
    const selected = structuredClone(accepted); selected.records_root = fresh;
    selected.paths.public_source = publicRoot; selected.paths.web_components = selectedRequest.paths.web_components;
    json("accepted-input.json", { ...selected, records_root: qualifiedRoot });
    json("conductor-plan.json", { contract: "ait.release.conductor-plan/v1", records_root: qualifiedRoot, release: request.release,
      source: { control_files: { [path.join(native, "ait-external.lock")]: hash(path.join(native, "ait-external.lock")) } } });
    json("public-final/receipt.json", { contract: "ait.release.public-source/v1", status: "complete", kind: "final", head_sha: f });
    write(qualifiedRoot, "public-final/head.txt", `${f}\n`); write(qualifiedRoot, "public-qualification/head.txt", `${q}\n`);
    json("conductor-state.json", { contract: "ait.release.conductor-state/v1", plan_sha256: "0".repeat(64), status: "failed" });
    const originalState = readFileSync(path.join(qualifiedRoot, "conductor-state.json"));
    const originalFiles = new Map(Object.keys(qualifiedOperationInputs(selected).files).map(file => [file, readFileSync(file)]));
    for (const mutate of [value => { value.paths.community_bin = service; }, value => { value.release.version = "1.2.1"; },
      value => { value.web.acceptance_snapshot = head; }, value => { value.records_root = path.join(qualifiedRoot, "nested"); }]) {
      const bad = structuredClone(selected); mutate(bad); assert.throws(() => qualifiedOperationInputs(bad));
    }
    for (const [name, mutate] of [["candidate.json", value => { value.qualification.workflow_control_commit = q; }],
      ["receipts.json", value => { value.pre_rc_admission.admission_sha256 = "0".repeat(64); }],
      ["prepare.json", value => { value.mutation.artifact_rebuild = true; }]]) {
      const file = path.join(qualifiedRoot, name), before = readFileSync(file), value = JSON.parse(before);
      mutate(value); writeFileSync(file, JSON.stringify(value)); assert.throws(() => qualifiedOperationInputs(selected)); writeFileSync(file, before);
    }
    const missing = path.join(qualifiedRoot, "receipts.json"), missingBytes = readFileSync(missing); rmSync(missing);
    assert.throws(() => qualifiedOperationInputs(selected)); writeFileSync(missing, missingBytes);
    write(publicRoot, "source.txt", "changed source\n"); assert.throws(() => qualifiedOperationInputs(selected));
    write(publicRoot, "source.txt", "1.2.0\n");
    const pinnedLock = path.join(native, "ait-external.lock"), lockBytes = readFileSync(pinnedLock);
    writeFileSync(pinnedLock, `${lockBytes}\n`); assert.throws(() => qualifiedOperationInputs(selected)); writeFileSync(pinnedLock, lockBytes);
    const providerCalls = path.join(root, "provider-calls.jsonl");
    write(native, "ci/release_operator.sh", `#!${process.execPath}\nconst fs=require('fs'),a=process.argv.slice(2);fs.appendFileSync(${JSON.stringify(providerCalls)},JSON.stringify(a)+'\\n');
if(a[0]==='bind-qualification'){const i=a.indexOf('--run-id');if(i<0)process.exit(99);a.splice(i,2,...${JSON.stringify(providerArgs)});
const r=require('child_process').spawnSync(${JSON.stringify(realOperator)},a,{stdio:'inherit'});process.exit(r.status??65);}
const names={'bind-qualification':'pre-rc-qualification.json',admit:'pre-tag-admission.json',prepare:'prepare.json','bind-receipts':'receipts.json','bind-candidate':'candidate.json'};
if(!names[a[0]])process.exit(99);fs.copyFileSync(${JSON.stringify(qualifiedRoot)}+'/'+names[a[0]],a[a.indexOf('--output')+1]);\n`, 0o755);
    write(native, "ci/release_authority_preflight.sh", `#!${process.execPath}\nrequire('fs').writeFileSync(process.argv[3],'{}');\n`, 0o755);
    write(native, "ci/release_export_source.sh", `#!${process.execPath}\nconst fs=require('fs');fs.mkdirSync(process.argv[4]+'/final/export-a',{recursive:true});fs.writeFileSync(process.argv[4]+'/final/export-a/source.txt','1.2.0\\n');\n`, 0o755);
    write(native, "ci/release_public_source.sh", `#!${process.execPath}\nconst fs=require('fs'),a=process.argv.slice(2);if(a[0]!=='probe-final')process.exit(99);
if(fs.readFileSync(a[2]+'/source.txt','utf8')!=='1.2.0\\n')process.exit(1);process.exit(fs.existsSync(a[4]+'/receipt.json')?0:1);\n`, 0o755);
    for (const name of readdirSync(here).filter(name => name.endsWith(".mjs"))) copyFileSync(path.join(here, name), path.join(native, "ci", name));
    copyFileSync(path.join(here, "release_candidate_materialize.sh"), path.join(native, "ci/release_candidate_materialize.sh"));
    chmodSync(path.join(native, "ci/release_candidate_materialize.sh"), 0o755);
    const qualifiedRequestPath = path.join(root, "qualified-request.json"); writeFileSync(qualifiedRequestPath, JSON.stringify(selectedRequest));
    const originalPublication = data.nativeLocal.publication_state; data.nativeLocal.publication_state = "unpublished"; persist();
    run("release_stable_prepare.mjs", ["plan", "--request", qualifiedRequestPath], 75);
    data.nativeLocal.publication_state = originalPublication; persist();
    run("release_stable_prepare.mjs", ["run", "--request", qualifiedRequestPath]);
    const freshAccepted = path.join(fresh, "accepted-input.json"), freshSpec = path.join(fresh, "release-spec.json");
    run("release_stable_spec.mjs", ["--input", freshAccepted, "--output", freshSpec]);
    const qualifiedRecipe = JSON.parse(readFileSync(path.join(here, "../release/stable-conductor.recipe.json")));
    const cleanHost = qualifiedRecipe.phases.find(phase => phase.id === "clean_host_qualification");
    const triple = `${process.arch === "arm64" ? "aarch64" : "x86_64"}-${process.platform === "darwin" ? "apple-darwin" : "unknown-linux-gnu"}`;
    cleanHost.actions[1] = { id: "cache-frozen-candidate", type: "command", mutation: false, cwd: root, outputs: [path.join(fresh, "candidate-cache")],
      argv: [process.execPath, "-e", `const fs=require('fs'),p=process.argv[1];fs.mkdirSync(p+'/payload',{recursive:true});
fs.writeFileSync(p+'/cache.json',JSON.stringify({artifact_id:104}));fs.writeFileSync(p+'/artifact.zip',Buffer.from(${JSON.stringify(zip.toString())}));
fs.copyFileSync(${JSON.stringify(ait)},p+'/payload/ait-1.2.0-${triple}');fs.copyFileSync(${JSON.stringify(service)},p+'/payload/ait-server-1.2.0-${triple}');`, path.join(fresh, "candidate-cache")] };
    const freshWeb = qualifiedRecipe.phases.find(phase => phase.id === "web_admission");
    freshWeb.actions[1] = { id: "await-native-acceptance", type: "evidence_gate", path: path.join(fresh, "actual-native-pass"), message: "retained Native acceptance required" };
    const freshRecipe = path.join(root, "qualified-recipe.json"); writeFileSync(freshRecipe, JSON.stringify(qualifiedRecipe));
    const freshPlan = path.join(fresh, "conductor-plan.json"), freshState = path.join(fresh, "conductor-state.json");
    const init = ["--spec", freshSpec, "--recipe", freshRecipe, "--plan", freshPlan];
    write(fresh, "conductor-checkpoints/copied.json", "{}");
    assert.match(run("release_conductor_init.mjs", init, 75).stderr, /rejects existing conductor state or checkpoints/);
    rmSync(path.join(fresh, "conductor-checkpoints"), { recursive: true });
    run("release_conductor_init.mjs", init);
    const frozenPlan = JSON.parse(readFileSync(freshPlan)); assert.equal(frozenPlan.phases.length, 14);
    assert(!frozenPlan.phases.filter(phase => ["public_qualification", "component_receipts", "clean_host_qualification"].includes(phase.id))
      .some(phase => phase.actions.some(action => action.type === "github_workflow")));
    const freshRun = ["run", "--plan", freshPlan, "--state", freshState];
    const planBytes = readFileSync(freshPlan);
    for (const mutate of [phase => { phase.actions.pop(); }, phase => { phase.actions[2].mutation = false; },
      phase => { phase.actions[2].argv[1] = "publish"; }, phase => { phase.actions[2].probe.argv[3] = path.join(root, "foreign-cache"); },
      phase => { phase.actions[1].mutation = true; phase.actions[1].probe = { cwd: root, argv: ["true"] }; },
      phase => { const argv = phase.actions[0].argv; argv[argv.indexOf("--run-id") + 1] = "999"; },
      phase => { phase.actions[0].argv[1] = "bind-receipts"; }]) {
      const changed = JSON.parse(planBytes); mutate(changed.phases.find(phase => phase.id === "clean_host_qualification"));
      writeFileSync(freshPlan, JSON.stringify(changed)); run("release_conductor.mjs", ["status", "--plan", freshPlan, "--state", freshState], 65);
    }
    writeFileSync(freshPlan, planBytes);
    run("release_conductor.mjs", freshRun, 75);
    const currentState = JSON.parse(readFileSync(freshState)), freshInput = path.join(fresh, "web-admission/qualification-input.json");
    validateWebOperation(freshInput, JSON.parse(readFileSync(freshInput)), webRoot);
    const freshBound = JSON.parse(readFileSync(freshInput));
    const materialized = currentState.phases.find(phase => phase.id === "clean_host_qualification").actions.find(action => action.id === "materialize-host-candidate");
    assert.equal(materialized.status, "complete");
    assert.deepEqual(readFileSync(freshBound.executables.ait.path), readFileSync(ait));
    assert.deepEqual(readFileSync(freshBound.executables.server.path), readFileSync(service));
    for (const name of ["conductor-checkpoints/bind-web-admission-input.json", "conductor-receipts/bind-web-admission-input.json"]) {
      const file = path.join(fresh, name), bytes = readFileSync(file); rmSync(file);
      assert.throws(() => validateWebOperation(freshInput, freshBound, webRoot)); writeFileSync(file, bytes);
    }
    data.nativeLocal.publication_state = "unpublished"; persist();
    assert.throws(() => validateWebOperation(freshInput, freshBound, webRoot));
    data.nativeLocal.publication_state = originalPublication; persist();
    const cachedZip = path.join(fresh, "candidate-cache/artifact.zip"), cachedBytes = readFileSync(cachedZip);
    writeFileSync(cachedZip, "changed binary artifact"); assert.throws(() => validateWebOperation(freshInput, freshBound, webRoot));
    writeFileSync(cachedZip, cachedBytes);
    assert.equal(currentState.phases.find(phase => phase.id === "web_admission").actions[0].status, "complete");
    assert.equal(currentState.phases.find(phase => phase.id === "web_admission").actions[1].status, "action_required");
    assert(!existsSync(path.join(fresh, "web-admission/receipts/foundation.json")));
    const provider = readFileSync(providerCalls, "utf8").trim().split("\n").map(line => JSON.parse(line));
    assert.deepEqual(provider.filter(args => args.includes("--run-id")).map(args => [args[0], args[args.indexOf("--run-id") + 1]]),
      [["bind-qualification", "101"], ["bind-receipts", "102"], ["bind-candidate", "103"]]);
    assert(provider.every(args => !args.includes("--dispatch")));
    run("release_conductor.mjs", freshRun, 75); assert.equal(readFileSync(providerCalls, "utf8").trim().split("\n").length, provider.length);
    const originalFreshState = readFileSync(freshState);
    writeFileSync(freshState, originalState); run("release_conductor.mjs", freshRun, 65); writeFileSync(freshState, originalFreshState);
    const manual = JSON.parse(originalFreshState); manual.phases.at(-1).actions[0].status = "complete";
    writeFileSync(freshState, JSON.stringify(manual)); assert.match(run("release_conductor.mjs", freshRun, 65).stderr, /without its native checkpoint/);
    writeFileSync(freshState, originalFreshState);
    const pinnedCandidate = path.join(qualifiedRoot, "candidate.json"), pinnedBytes = readFileSync(pinnedCandidate);
    writeFileSync(pinnedCandidate, `${pinnedBytes}\n`); assert.match(run("release_conductor.mjs", freshRun, 65).stderr, /control source drifted/);
    writeFileSync(pinnedCandidate, pinnedBytes);
    for (const [file, bytes] of originalFiles) assert.deepEqual(readFileSync(file), bytes);
    assert.deepEqual(readFileSync(path.join(qualifiedRoot, "conductor-state.json")), originalState);
    const invocations = readFileSync(calls, "utf8").trim().split("\n").map(row => JSON.parse(row));
    assert(invocations.some(({ a }) => a[0] === "patchset" && a[2] === corePublic));
    assert(invocations.every(({ a }) => a.every(arg => !/[A-Z]+T-[0-9]{4}\/C-[0-9]{2}/.test(arg))));
    assert(invocations.every(({ a }) => !a.includes("--apply") && !["start", "create", "push", "update", "install", "build"].some(word => a.includes(word))));
    assert(!existsSync(sources.server.edit_root)); // Cleaned roots never become source workspaces.
  } finally { server.kill(); }
}

try {
  await admittedBootstrap();
  // Validate the approved source-request triple through the actual parser,
  // without running preparation, credentials, services or product commands.
  const requestBinary = path.join(temporary, "request-binary"); writeFileSync(requestBinary, "fixture");
  const sourceRequest = { contract: "ait.release.request/v1", release: { channel: "stable", prior_version: "1.1.4", version: "1.2.0" },
    records_root: path.join(temporary, "scope-records"), roots: Object.fromEntries(["core", "server", "runner", "python", "node", "native"].map(key => [key, temporary])),
    paths: { public_source: temporary, web_test_root: temporary, community_root: temporary, community_bin: requestBinary, community_cli_bin: requestBinary, chrome: requestBinary },
    web: { server_url: "http://127.0.0.1:8088/", server_data: temporary, seed: 42, acceptance_snapshot: pin, community_snapshot: pin },
    github_repository: "owner/repository", winget_fork: "owner/winget-pkgs", server_refresh: { stop_argv: ["fixture-stop"], start_argv: ["fixture-start"], health_url: "http://127.0.0.1:8088/healthz" } };
  const requestFile = path.join(temporary, "scope-request.json");
  const checkRequest = value => {
    writeFileSync(requestFile, JSON.stringify(value));
    return spawnSync(process.execPath, ["--input-type=module", "-e",
      `import { validateRequest } from ${JSON.stringify(new URL("./release_stable_prepare.mjs", import.meta.url).href)}; const {request}=validateRequest(process.argv[1]); console.log(Object.hasOwn(request.paths,"personal_bin")?"full":"retained");`, requestFile], { encoding: "utf8" });
  };
  assert.equal(checkRequest(sourceRequest).stdout.trim(), "retained");
  const pausedRequest = structuredClone(sourceRequest);
  for (const key of ["community_root", "community_bin", "community_cli_bin", "chrome"]) delete pausedRequest.paths[key];
  delete pausedRequest.web.community_snapshot;
  assert.equal(checkRequest(pausedRequest).status, 0);
  for (let mask = 1; mask < 15; mask++) {
    const partial = structuredClone(pausedRequest);
    ["community_root", "community_bin", "community_cli_bin", "chrome"].forEach((key, index) => { if (mask & (1 << index)) partial.paths[key] = key === "community_root" ? temporary : requestBinary; });
    assert.notEqual(checkRequest(partial).status, 0);
  }
  for (const value of [undefined, null, "", "SNP-INVALID"]) {
    const changed = structuredClone(sourceRequest); changed.web.community_snapshot = value;
    assert.notEqual(checkRequest(changed).status, 0);
  }
  const orphan = structuredClone(pausedRequest); orphan.web.community_snapshot = pin;
  assert.notEqual(checkRequest(orphan).status, 0);
  for (const version of ["1.1.4", "1.2.1"]) {
    const changed = structuredClone(pausedRequest); changed.release = { channel: "stable", prior_version: version === "1.1.4" ? "1.1.3" : "1.2.0", version };
    assert.notEqual(checkRequest(changed).status, 0);
  }
  const fullRequest = structuredClone(sourceRequest); fullRequest.paths.personal_root = temporary;
  fullRequest.paths.personal_bin = requestBinary; fullRequest.web.personal_snapshot = pin;
  assert.equal(checkRequest(fullRequest).stdout.trim(), "full");
  for (let mask = 1; mask < 7; mask++) {
    const partial = structuredClone(sourceRequest);
    if (mask & 1) partial.paths.personal_root = temporary;
    if (mask & 2) partial.paths.personal_bin = requestBinary;
    if (mask & 4) partial.web.personal_snapshot = pin;
    assert.notEqual(checkRequest(partial).status, 0);
  }
  for (const invalid of [null, ""]) for (const [group, key] of [["paths", "personal_root"], ["paths", "personal_bin"], ["web", "personal_snapshot"]]) {
    const changed = structuredClone(fullRequest); changed[group][key] = invalid;
    assert.notEqual(checkRequest(changed).status, 0);
  }
  assert.equal(stableAdvance("1.1.2", "1.1.3"), true);
  assert.equal(stableAdvance("1.1.3", "1.2.0"), true);
  assert.equal(stableAdvance("1.1.2", "1.1.4"), false);

  const binding = bindingCard(version);
  assert.equal(binding.relative, "docs/sprints/release_1_1_3_binding.md");
  assert.equal(binding.ref, "release-1-1-3/external-binding");
  assert.match(binding.content, /family template, bootstrap matrix, repository authorities, ait-external\.toml, and ait-external\.lock/);
  assert.deepEqual(bindingCard(version), binding);

  const closeoutRoots = Object.fromEntries(
    ["core", "server", "runner", "python", "node"].map((component) => [
      component,
      path.join(temporary, component),
    ]),
  );
  const sourceComponents = Object.fromEntries(
    ["core", "server", "runner", "python", "node"].map((component, index) => [
      component,
      {
        task: `${component === "core" ? "LC" : "R"}T-${String(1100 + index).padStart(4, "0")}`,
        snapshot: `SNP-${String(index + 1).repeat(12)}`,
        edit_root: path.join(temporary, `source-${component}`),
        stage: component === "core" || component === "server" ? "finished_local" : "snapshot_ready",
      },
    ]),
  );
  const closeouts = acceptedComponentCloseouts(
    { roots: closeoutRoots },
    { components: sourceComponents },
  );
  assert.deepEqual(
    closeouts.map(({ id, task, snapshot, finish_local_before_ready }) => ({
      id,
      task,
      snapshot,
      finish_local_before_ready,
    })),
    [
      { id: "core", task: sourceComponents.core.task, snapshot: sourceComponents.core.snapshot, finish_local_before_ready: true },
      { id: "server", task: sourceComponents.server.task, snapshot: sourceComponents.server.snapshot, finish_local_before_ready: true },
      { id: "runner", task: sourceComponents.runner.task, snapshot: sourceComponents.runner.snapshot, finish_local_before_ready: true },
      { id: "python", task: sourceComponents.python.task, snapshot: sourceComponents.python.snapshot, finish_local_before_ready: true },
      { id: "node", task: sourceComponents.node.task, snapshot: sourceComponents.node.snapshot, finish_local_before_ready: true },
    ],
  );
  assert.equal(closeouts[0].snapshot, "SNP-111111111111");
  const readiness = closeoutReadinessProbes("/exact/ait", closeouts);
  assert.deepEqual(
    readiness.map(({ id, cwd, argv }) => ({ id, cwd, argv })),
    closeouts.map((row) => ({
      id: `preflight-closeout-${row.id}`,
      cwd: row.finish_local_before_ready ? row.repository_root : row.edit_root,
      argv: ["/exact/ait", "workflow", "ready", row.task, "--remote", "origin"],
    })),
  );
  assert.equal(readiness.some(({ argv }) => argv.includes("--apply")), false);

  const core = path.join(temporary, "core");
  write(core, "rust/Cargo.toml", repeated(oldVersion, 1));
  write(core, "rust/Cargo.lock", packageLock(["ait-agent-core", "ait-agent-worker", "ait-cli", "ait-napi", "ait-py"]) + `\n[[package]]\nname = "ait-core"\nversion = "0.1.0"\n\n[[package]]\nname = "atomic-waker"\nversion = "${oldVersion}"\n`);
  for (const [relative, amount] of [["ait-release.json", 1], ["ait-release-family.json", 9], ["ci/native_bootstrap_matrix.json", 1], ["ci/release_repository_authorities.json", 1]]) {
    write(core, relative, repeated(oldVersion, amount));
  }
  const dryRunCoreFiles = ["rust/Cargo.toml", "rust/Cargo.lock", "ait-release.json", "ait-release-family.json", "ci/native_bootstrap_matrix.json", "ci/release_repository_authorities.json"];
  const dryRunCoreBytes = new Map(dryRunCoreFiles.map((relative) => [relative, readFileSync(path.join(core, relative))]));
  assert.equal(replaceComponentSources(core, "core", oldVersion, version, null, false).length, 6);
  const native = path.join(temporary, "native");
  for (const [relative, amount] of [["ait-release-family.json", 7], ["ci/native_bootstrap_matrix.json", 1], ["ci/release_repository_authorities.json", 1]]) {
    write(native, relative, repeated(oldVersion, amount));
  }
  write(native, "ait-external.toml", `[[external]]\nname = "ait-core"\nversion = "${oldVersion}"\n`);
  const pinnedRunnerRow = `[[node]]\nname = "ait-core"\nparent_path = ".ait-external/ait-runner"\nversion = "0.9.9"\n`;
  const familyCoreRows = `[[node]]\nname = "ait-core"\nparent_path = ""\nversion = "${oldVersion}"\n\n[[node]]\nname = "ait-core"\nparent_path = ".ait-external/ait-node"\nversion = "${oldVersion}"\n\n`;
  write(native, "ait-external.lock", familyCoreRows + pinnedRunnerRow);
  assert.deepEqual(replaceComponentSources(native, "native", oldVersion, version, null), ["ait-release-family.json", "ci/native_bootstrap_matrix.json", "ci/release_repository_authorities.json", "ait-external.toml", "ait-external.lock"]);
  assert.doesNotMatch(readFileSync(path.join(native, "ait-release-family.json"), "utf8"), /1\.1\.2/);
  assert.equal(readFileSync(path.join(native, "ait-external.lock"), "utf8"), familyCoreRows.replaceAll(oldVersion, version) + pinnedRunnerRow);
  replaceComponentSources(native, "native", version, oldVersion, null);
  assert.equal(readFileSync(path.join(native, "ait-external.lock"), "utf8"), familyCoreRows + pinnedRunnerRow);
  const bareCore = path.join(temporary, "bare-core");
  write(bareCore, "rust/Cargo.toml", repeated(oldVersion, 1));
  write(bareCore, "rust/Cargo.lock", packageLock(["ait-agent-core", "ait-agent-worker", "ait-cli", "ait-napi", "ait-py"]));
  write(bareCore, "ait-release.json", repeated(oldVersion, 1));
  assert.deepEqual(replaceComponentSources(bareCore, "core", oldVersion, version), ["ait-release.json", "rust/Cargo.lock", "rust/Cargo.toml"]);
  for (const [relative, bytes] of dryRunCoreBytes) assert.deepEqual(readFileSync(path.join(core, relative)), bytes);
  assert.equal(replaceComponentSources(core, "core", oldVersion, version).length, 6);
  const forwardCoreLock = readFileSync(path.join(core, "rust/Cargo.lock"), "utf8");
  assert.match(forwardCoreLock, /ait-napi"\nversion = "1\.1\.3"/);
  assert.match(forwardCoreLock, /ait-core"\nversion = "0\.1\.0"/);
  assert.match(forwardCoreLock, /atomic-waker"\nversion = "1\.1\.2"/);
  assert.equal(replaceComponentSources(core, "core", version, oldVersion).length, 6);
  const reverseCoreLock = readFileSync(path.join(core, "rust/Cargo.lock"), "utf8");
  assert.match(reverseCoreLock, /ait-napi"\nversion = "1\.1\.2"/);
  assert.match(reverseCoreLock, /ait-core"\nversion = "0\.1\.0"/);

  const server = path.join(temporary, "server");
  for (const relative of ["rust/crates/ait-server/Cargo.toml", "ait-release.json", "rust/crates/ait-server/src/router/http/binary_tests.rs"]) write(server, relative, repeated(oldVersion, 1));
  write(server, "rust/Cargo.lock", packageLock(["ait-server"]));
  assert.equal(replaceComponentSources(server, "server", oldVersion, version).length, 4);

  const runner = path.join(temporary, "runner");
  write(runner, "Cargo.toml", repeated(oldVersion, 1));
  write(runner, "Cargo.lock", packageLock(["ait-runner"]));
  write(runner, "ait-release.json", repeated(oldVersion, 1));
  write(runner, "ait-external.toml", external());
  write(runner, "ait-external.lock", external());
  write(runner, "tests/cli.rs", repeated(oldPin, 1));
  const runnerManifest = readFileSync(path.join(runner, "ait-external.toml"));
  const runnerLock = readFileSync(path.join(runner, "ait-external.lock"));
  const runnerCliTest = readFileSync(path.join(runner, "tests/cli.rs"));
  assert.equal(replaceComponentSources(runner, "runner", oldVersion, version, pin).length, 3);
  assert.deepEqual(readFileSync(path.join(runner, "ait-external.toml")), runnerManifest);
  assert.deepEqual(readFileSync(path.join(runner, "ait-external.lock")), runnerLock);
  assert.deepEqual(readFileSync(path.join(runner, "tests/cli.rs")), runnerCliTest);

  const python = path.join(temporary, "python");
  const pythonCounts = new Map([["pyproject.toml", 1], ["ait-release.json", 7], ["src/ait_python/__init__.py", 1], ["tests/test_package.py", 3], ["tests/test_runtime.py", 1]]);
  for (const [relative, amount] of pythonCounts) write(python, relative, repeated(oldVersion, amount));
  write(python, "ait-external.toml", external());
  write(python, "ait-external.lock", external());
  write(python, "tests/test_package.py", repeated(oldVersion, 3) + repeated(oldPin, 1));
  const pythonAdapterTest = `assert manifest["package"]["version"] == "${oldVersion}"\nexpected_wheel_name(target, '${oldVersion}')\nhistorical = "1.1.4"\nsynthetic = "1.2.0"\n`;
  write(python, "tests/test_release_adapter.py", pythonAdapterTest);
  assert.equal(replaceComponentSources(python, "python", oldVersion, version, pin).length, 8);
  assert.match(readFileSync(path.join(python, "tests/test_release_adapter.py"), "utf8"), /version"\] == "1\.1\.3"/);
  assert.match(readFileSync(path.join(python, "tests/test_release_adapter.py"), "utf8"), /historical = "1\.1\.4"\nsynthetic = "1\.2\.0"/);
  write(python, "tests/test_release_adapter.py", pythonAdapterTest + `assert manifest["package"]["version"] == "${oldVersion}"\n`);
  assert.throws(() => replaceComponentSources(python, "python", oldVersion, version, pin, false), /assertion inventory differs/);
  write(python, "tests/test_release_adapter.py", pythonAdapterTest.replaceAll(oldVersion, version));

  const node = path.join(temporary, "node");
  const nodeCounts = new Map([
    ["package.json", 7], ["ait-release.json", 8], ["lib/npm-payload-contract.json", 7],
    ["test/package.test.js", 1], ["test/release-adapter.test.js", 9], ["test/runtime.test.js", 2],
    ["ci/run.ps1", 2], ["ci/run.sh", 2], ["release/release-adapter.mjs", 2], ["release/npm-payload-package.mjs", 1],
    ["scripts/installed-smoke.mjs", 1], ["scripts/native-build.mjs", 1], ["src/runtime.js", 1],
  ]);
  for (const [relative, amount] of nodeCounts) write(node, relative, repeated(oldVersion, amount));
  write(node, "ait-external.toml", external());
  write(node, "ait-external.lock", external());
  write(node, "test/package.test.js", repeated(oldVersion, 1) + repeated(oldVersion.replaceAll(".", "\\."), 1) + repeated(oldPin, 2));
  write(node, "test/release-adapter.test.js", repeated(oldVersion, 9) + repeated(oldVersion.replaceAll(".", "\\."), 2));
  for (const [relative, amount] of new Map([["lib/npm-payload-contract.json", 6], ["ci/run.ps1", 1], ["ci/run.sh", 1], ["release/release-adapter.mjs", 1], ["release/npm-payload-package.mjs", 1], ["scripts/native-build.mjs", 1], ["src/runtime.js", 1]])) {
    write(node, relative, readFileSync(path.join(node, relative), "utf8") + repeated(oldPin, amount));
  }
  const retirementMessage = "Chat integrations are retired in AIT 1.2.0";
  for (const relative of ["release/release-adapter.mjs", "scripts/installed-smoke.mjs"]) {
    write(node, relative, readFileSync(path.join(node, relative), "utf8") + retirementMessage + "\n");
  }
  const payloadFixture = `${retirementMessage}\nhistorical = "1.1.4"\nwrongVersion = "1.1.4-rc.99"\n`;
  write(node, "test/payload-package.test.js", payloadFixture);
  assert.equal(replaceComponentSources(node, "node", oldVersion, version, pin).length, 15);
  assert.doesNotMatch(readFileSync(path.join(node, "lib/npm-payload-contract.json"), "utf8"), /1\.1\.2|SNP-111111111111/);
  assert.equal(readFileSync(path.join(node, "test/payload-package.test.js"), "utf8"), payloadFixture);
  replaceComponentSources(node, "node", version, "1.2.0", pin);
  const nodeAtRelease = new Map([...nodeCounts.keys(), "ait-external.toml", "ait-external.lock", "test/payload-package.test.js"]
    .map(relative => [relative, readFileSync(path.join(node, relative))]));
  replaceComponentSources(node, "node", "1.2.0", "1.1.4", pin);
  for (const relative of ["release/release-adapter.mjs", "scripts/installed-smoke.mjs"]) {
    assert(readFileSync(path.join(node, relative), "utf8").includes(retirementMessage));
  }
  assert.equal(readFileSync(path.join(node, "test/payload-package.test.js"), "utf8"), payloadFixture);
  replaceComponentSources(node, "node", "1.1.4", "1.2.0", pin);
  for (const [relative, bytes] of nodeAtRelease) assert.deepEqual(readFileSync(path.join(node, relative)), bytes);

  const bad = path.join(temporary, "bad-core");
  write(bad, "rust/Cargo.toml", repeated(oldVersion, 1));
  write(bad, "rust/Cargo.lock", packageLock(["ait-agent-core", "ait-agent-worker", "ait-cli", "ait-napi", "ait-py"]) + `\n[[package]]\nname = "ait-core"\nversion = "0.1.0"\n`);
  for (const [relative, amount] of [["ait-release.json", 1], ["ait-release-family.json", 8], ["ci/native_bootstrap_matrix.json", 1], ["ci/release_repository_authorities.json", 1]]) write(bad, relative, repeated(oldVersion, amount));
  assert.throws(() => replaceComponentSources(bad, "core", oldVersion, version), /token inventory differs/);

  const retainedSource = path.join(temporary, "retained-source.json");
  const retainedCopy = path.join(temporary, "retained-copy.json");
  writeFileSync(retainedSource, "{\"stable\":true}\n");
  copyOrVerify(retainedSource, retainedCopy, "retained test copy");
  copyOrVerify(retainedSource, retainedCopy, "retained test copy");
  assert.equal(readFileSync(retainedCopy, "utf8"), "{\"stable\":true}\n");
  const retainedText = path.join(temporary, "retained-text.json");
  writeOrVerify(retainedText, "{\"status\":\"pass\"}\n", "retained test text");
  writeOrVerify(retainedText, "{\"status\":\"pass\"}\n", "retained test text");

  const stagedDirectory = path.join(temporary, "staged-directory");
  let materializations = 0;
  const materialize = () => materializeDirectory(stagedDirectory, (pending) => {
    materializations += 1;
    mkdirSync(pending, { recursive: true });
    writeFileSync(path.join(pending, "receipt.json"), "{\"status\":\"pass\"}\n");
  }, (candidate) => assert.equal(readFileSync(path.join(candidate, "receipt.json"), "utf8"), "{\"status\":\"pass\"}\n"));
  materialize();
  materialize();
  assert.equal(materializations, 1, "retained directory must resume without rerunning its producer");

  const interruptedDirectory = path.join(temporary, "interrupted-directory");
  mkdirSync(`${interruptedDirectory}.pending`, { recursive: true });
  writeFileSync(path.join(`${interruptedDirectory}.pending`, "partial"), "partial\n");
  materializeDirectory(interruptedDirectory, (pending) => {
    mkdirSync(pending, { recursive: true });
    writeFileSync(path.join(pending, "complete"), "complete\n");
  }, (candidate) => assert.equal(readFileSync(path.join(candidate, "complete"), "utf8"), "complete\n"));
  assert.equal(existsSync(path.join(interruptedDirectory, "partial")), false);
  assert.equal(readFileSync(path.join(interruptedDirectory, "complete"), "utf8"), "complete\n");

  const source = readFileSync(path.join(here, "release_stable_prepare.mjs"), "utf8");
  // Controller execution cannot take the independent Reviewer's Local finish.
  assert.doesNotMatch(source, /"task", "finish"/);
  assert.match(source, /state\.components\[component\] = row;\n\s+save\(\);/);
  assert.match(source, /row\.progress = "finalizing";\n\s+save\(\);/);
  assert.doesNotMatch(source, /coordinator|freeze_prior|equivalence|normalize\.mjs/i);
  assert.match(source, /if \(!done\("pin_externals"\)\)/);
  for (const component of ["core", "server", "runner", "python", "node"]) {
    assert.match(source, new RegExp(`publishComponentSource\\("${component}", ${component}\\)`));
  }
  assert.ok(source.indexOf('publishComponentSource("node", node)') < source.indexOf('if (!done("pin_externals"))'));
  assert.match(source, /"external", "update", repository, "--to", snapshot, "--no-recursive"/);
  assert.match(source, /replaceComponentSources\(binding\.edit_root, "native"/);
  assert.match(source, /binding\.progress = "finalizing";\n\s+save\(\);/);
  assert.match(source, /native_root: request\.roots\.native/);
  const example = JSON.parse(readFileSync(path.join(here, "../release/stable-request.example.json"), "utf8"));
  assert.equal(example.paths.web_components, undefined);
  assert.equal(typeof example.roots.native, "string");
  process.stdout.write("release stable preparation tests passed\n");
} finally {
  rmSync(temporary, { recursive: true, force: true });
}

// Isolated producer integration. AIT authority, hosted qualification and Cargo
// execution are protocol fixtures, never actual release or approval evidence.
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { chmodSync, copyFileSync, cpSync, existsSync, mkdirSync, readFileSync, renameSync, symlinkSync, writeFileSync } from "node:fs";
import { spawn, spawnSync } from "node:child_process";
import path from "node:path";
import { previousCohort, correctedCohort, cacheDirectory } from "./release_web_applicability.mjs";
import { installedCoreCases } from "./release_core_installed_cases.mjs";

export async function runCorrectedCohortFixture(c) {
  const { root, here, native, nativeCi, repository, original, cli, server, retainedInit, collection, recoveryRecipe, protocolBin, protocolState, zip } = c;
  const put = (file, bytes) => { mkdirSync(path.dirname(file), { recursive: true }); writeFileSync(file, bytes); };
  const save = (file, value) => put(file, JSON.stringify(value) + "\n");
  const read = file => JSON.parse(readFileSync(file));
  const sha = file => createHash("sha256").update(readFileSync(file)).digest("hex");
  const source = path.join(root, "approved-public"), records = path.join(root, "approved-operation"), dataRoot = path.join(root, "approved-provider");
  mkdirSync(source); mkdirSync(records); mkdirSync(dataRoot);
  const git = args => { const result = spawnSync("git", ["-C", source, ...args], { encoding: "utf8" }); assert.equal(result.status, 0, result.stderr); return result.stdout.trim(); };
  git(["init", "-q"]); git(["config", "user.name", "Fixture"]); git(["config", "user.email", "fixture@example.test"]);
  const family = version => ({ schema: "ait.release.family/v3", family: { version, tag: "v" + version },
    components: ["core", "node", "python", "server", "runner"].map(id => ({ source_repository: "ait-" + id, source_snapshot: correctedCohort[id] })) });
  const sourceFiles = version => {
    const document = family(version); save(path.join(source, "ait-release-family.json"), document);
    save(path.join(source, "ait-monorepo-source.json"), { schema: "ait.release.monorepo-source/v1", family_version: version, family_tag: "v" + version,
      family_manifest_sha256: sha(path.join(source, "ait-release-family.json")), subtrees: document.components.map(row => ({ ...row, path: row.source_repository })) });
    save(path.join(source, "ait-core/ait-release.json"), { package: { name: "ait-core", version } });
  };
  sourceFiles("1.1.4"); git(["add", "."]); git(["commit", "-qm", "source qualification fixture"]); const qCommit = git(["rev-parse", "HEAD"]);
  sourceFiles("1.2.0"); git(["add", "."]); git(["commit", "-qm", "version-only fixture"]); const commit = git(["rev-parse", "HEAD"]);
  const paths = {
    core: ["src/agent_teams/activation.rs", "src/agent_teams/mod.rs", "src/binding_output.rs", "src/binding_team.rs", "src/config_surface.rs", "tests/team_bootstrap_cli.rs", "tests/team_caller_activation_cli.rs"].map(file => "rust/crates/ait-cli/" + file),
    node: ["ait-external.lock", "ait-external.toml", "ci/run.ps1", "ci/run.sh", "lib/npm-payload-contract.json", "release/npm-payload-package.mjs", "release/release-adapter.mjs", "scripts/native-build.mjs", "src/runtime.js", "test/package.test.js"],
    python: ["ait-external.lock", "ait-external.toml", "tests/test_package.py"],
  };
  const diffs = Object.fromEntries(Object.entries(paths).map(([id, files]) => [previousCohort[id] + ":" + correctedCohort[id], {
    old_snapshot_id: previousCohort[id], new_snapshot_id: correctedCohort[id], added: [], deleted: [], mode_changed: [], rename_hints: [], directory_move_hints: [],
    files: files.map(file => ({ path: file, status: "modified", diff: { status: "text", text: `--- old\n+++ new\n-snapshot = "${previousCohort.core}"\n+snapshot = "${correctedCohort.core}"\n` } })),
  }]));
  const diffFile = path.join(root, "approved-diff-fixture.json"); save(diffFile, diffs);

  const newCli = path.join(dataRoot, "ait-new"); put(newCli, readFileSync(cli, "utf8") + "\n// corrected candidate fixture\n"); chmodSync(newCli, 0o755);
  const qBody = { contract: "ait.release.pre-rc-qualification/v1", status: "qualified", source: { git_commit: qCommit, workflow_control_commit: qCommit },
    gates: Object.fromEntries(["core_quality", "command_inventory", "environment_inventory", "release_controls"].map(key => [key, "pass"])),
    immutable_release_tag_created: false, public_endpoint_writes: false, release_receipts_created: false };
  const qZip = path.join(dataRoot, "q.zip"), dZip = path.join(dataRoot, "d.zip"), eZip = path.join(dataRoot, "e.zip");
  zip(qZip, { "ait-release.pre-rc-qualification.json": qBody });
  const release = { id: "REL-FAM-2222222222222222", version: "1.2.0", channel: "stable", prior_version: "1.1.4", source_commit: commit, repository: "owner/repository" };
  zip(dZip, { "fixture-candidate.json": { release, components: correctedCohort } }); const dDigest = "sha256:" + sha(dZip);
  const eRelease = { ...release, candidate_artifact_digest: dDigest }, members = {}, inventory = [];
  const targets = ["aarch64-unknown-linux-gnu", "x86_64-unknown-linux-gnu", "aarch64-pc-windows-msvc", "x86_64-pc-windows-msvc"];
  for (let index = 0; index < 24; index++) {
    const target = targets[index], file = target ? `rows/github-product-${target}.json` : `rows/package-fixture-${index}.json`;
    const row = { status: "pass", failures: [], release: eRelease, row: { target: target ?? "fixture-" + index } };
    if (target) row.phases = { install: { status: "pass", observations: { installed_core_cases: { core_snapshot: correctedCohort.core,
      source_commit: commit, cases: installedCoreCases.map(([target, selector, mode]) => ({ target, selector, mode: mode ?? "fixture" })) } },
    commands: installedCoreCases.map(([suite, selector]) => ({ label: `installed Core ${suite}/${selector}`, exit_status: 0,
      stdout_tail: `running 1 test\ntest ${selector} ... ok\ntest result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 1 filtered out; finished in 0.1s\n` })) } };
    members[file] = row; inventory.push({ path: file, sha256: createHash("sha256").update(JSON.stringify(row) + "\n").digest("hex") });
  }
  const aggregate = { contract: "ait.release.clean-host.aggregate/v1", status: "qualified", failures: [], release: eRelease, evidence_inventory: inventory };
  members["ait-release.clean-host-status.json"] = aggregate; zip(eZip, members);
  const qBinding = { contract: "ait.release.operator.qualification-binding/v1", status: "qualified_for_version_only_release", source: { git_commit: qCommit },
    workflow: { run_id: 8001, run_attempt: 1, artifact_id: 8101, artifact_digest: "sha256:" + sha(qZip),
      evidence_sha256: createHash("sha256").update(JSON.stringify(qBody) + "\n").digest("hex"), control_commit: qCommit } };
  const candidate = { contract: "ait.release.operator.pre-tag-candidate-binding/v1", status: "ready_for_immutable_tag", release,
    candidate_authority: { sha256: "b".repeat(64), record: { prior_release: { version: "1.1.4" } } }, qualification: {
      workflow_run_id: 8002, workflow_run_attempt: 1, workflow_control_commit: commit, clean_host_rows: 24, tag_state_at_closeout: "absent",
      candidate_artifact_id: 8102, candidate_artifact_digest: dDigest, aggregate_artifact_id: 8103, aggregate_artifact_digest: "sha256:" + sha(eZip),
      aggregate_status_sha256: createHash("sha256").update(JSON.stringify(aggregate) + "\n").digest("hex") } };
  const qFile = path.join(dataRoot, "pre-rc-qualification.json"), candidateFile = path.join(dataRoot, "candidate.json"); save(qFile, qBinding); save(candidateFile, candidate);

  // Existing full qualification is an external execution boundary in this
  // fixture. Production calls the unchanged real 24-row/package verifier.
  put(path.join(nativeCi, "release_prepublish_verify.mjs"), `import assert from 'node:assert/strict';import fs from 'node:fs';const a=process.argv.slice(2),get=k=>a[a.indexOf(k)+1];assert.equal(a[0],'qualify');const e=JSON.parse(fs.readFileSync(get('--root')+'/ait-release.clean-host-status.json'));const d=JSON.parse(fs.readFileSync(get('--candidate-root')+'/fixture-candidate.json'));assert.equal(e.evidence_inventory.length,24);assert.equal(d.components.core,'${correctedCohort.core}');assert.equal(e.release.source_commit,d.release.source_commit);\n`);
  copyFileSync(path.join(here, "release_artifact_cache.sh"), path.join(nativeCi, "release_artifact_cache.sh")); chmodSync(path.join(nativeCi, "release_artifact_cache.sh"), 0o755);
  put(path.join(nativeCi, "release_candidate_materialize.sh"), `#!${process.execPath}\nconst fs=require('fs'),path=require('path'),a=process.argv.slice(2),out=a[3];if(a[0]==='probe'){if(!fs.existsSync(out+'/ait'))process.exit(1);if(!fs.readFileSync(out+'/ait').equals(fs.readFileSync(${JSON.stringify(newCli)})))process.exit(2);process.exit(0);}if(a[0]!=='materialize')process.exit(64);fs.mkdirSync(out,{recursive:true});fs.copyFileSync(${JSON.stringify(newCli)},out+'/ait');fs.chmodSync(out+'/ait',0o755);fs.copyFileSync(${JSON.stringify(server)},out+'/ait-server');fs.chmodSync(out+'/ait-server',0o755);fs.writeFileSync(${JSON.stringify(path.join(root, "expected-installed-cli"))},out+'/ait\\n');\n`); chmodSync(path.join(nativeCi, "release_candidate_materialize.sh"), 0o755);

  const roots = Object.fromEntries(["core", "node", "python", "server", "runner"].map(id => [id, path.join(root, "approved-repositories", "ait-" + id)])); roots.native = native;
  for (const directory of Object.values(roots)) mkdirSync(directory, { recursive: true });
  const ids = { core: 0, server: 1, python: 2, node: 3, runner: 4 }, tasks = { core: "RCT-1939", node: "RNT-0096", python: "RPT-0089", server: "RSET-0698", runner: "RRT-0070" };
  const declarations = Object.entries(ids).map(([id, index]) => ({ name: "ait-" + id, repo_name: "ait-" + id, repository_index: index,
    remote: "origin", line: "main", snapshot: correctedCohort[id], materialize_to: ".ait-external/ait-" + id,
    license: id === "server" ? "AGPL-3.0-only" : "Apache-2.0", version: "1.2.0" }));
  const toml = (table, rows) => rows.map(row => `[[${table}]]\n` + Object.entries(row).map(([key, value]) => `${key} = ${JSON.stringify(value)}\n`).join("")).join("\n");
  const nested = ["node", "python", "runner"].map(id => ({ name: "ait-core", repo_name: "ait-core", repository_index: 0, remote: "origin", line: "main",
    snapshot: id === "runner" ? "SNP-B1E5120FA441" : correctedCohort.core, parent_path: ".ait-external/ait-" + id,
    materialize_to: `.ait-external/ait-${id}/.ait-external/ait-core`, license: "Apache-2.0", version: id === "runner" ? "1.1.4" : "1.2.0" }));
  put(path.join(native, "ait-external.toml"), toml("external", declarations));
  put(path.join(native, "ait-external.lock"), 'format = "ait.external.lock"\n\n' + toml("node", [...declarations.map(row => ({ ...row, parent_path: "" })), ...nested]));
  // This protocol fixture keeps the admitted historical cohort even while
  // the owning repository prepares a later family version.
  const nativeFamily = read(path.join(here, "../ait-release-family.json"));
  nativeFamily.family.version = release.version;
  nativeFamily.family.tag = "v" + release.version;
  for (const component of nativeFamily.components) component.version = release.version;
  save(path.join(native, "ait-release-family.json"), nativeFamily);
  const nativeAdapter = read(path.join(here, "../ait-release.json"));
  nativeAdapter.package.version = release.version;
  save(path.join(native, "ait-release.json"), nativeAdapter);
  for (const row of nested) save(path.join(native, row.materialize_to, ".ait-external-marker.json"), { format: "ait.external.materialized", ...row });
  const runnerCore = { ...nested[2], parent_path: "", materialize_to: ".ait-external/ait-core" };
  put(path.join(native, ".ait-external/ait-runner/ait-external.toml"), toml("external", [runnerCore]));
  put(path.join(native, ".ait-external/ait-runner/ait-external.lock"), toml("node", [runnerCore]));
  const remoteTask = (task, snapshot, repo, index, number = 1) => ({ task_id: task, repo_name: repo, repository_index: index, status: "completed", workflow_state: "completed",
    current_patchset_id: `${task}/P-${String(number).padStart(2, "0")}`, successful_land: { task_id: task, repo_name: repo, repository_index: index,
      status: "succeeded", governance_authority: true, target_line: "main", revision_snapshot_id: snapshot, landed_snapshot_id: snapshot,
      patchset_id: `${task}/P-${String(number).padStart(2, "0")}`, submission_id: `${task}/L-01`, change_id: null, change_ref: null,
      result: { selected_revision_snapshot_id: snapshot, landed_snapshot_id: snapshot, target_line: "main" } } });
  const nativeHead = "SNP-3F43F5F85B18", nativeLocal = { task_id: "LNAT-0060", repo_name: "ait-native", status: "completed",
    publication_state: "published", published_remote_name: "origin", published_task_id: "RNAT-0042",
    local_finish: { task_id: "LNAT-0060", status: "succeeded", target_line: "main", landed_snapshot_id: nativeHead } };
  const remote = Object.fromEntries(Object.entries(tasks).map(([id, task]) => [task, remoteTask(task, correctedCohort[id], "ait-" + id, ids[id])]));
  remote["RNAT-0042"] = remoteTask("RNAT-0042", nativeHead, "ait-native", 28, 2);
  const patchsets = Object.fromEntries(Object.values(remote).map(task => [task.current_patchset_id, { task_id: task.task_id, patchset_id: task.current_patchset_id,
    patchset_number: Number(task.current_patchset_id.slice(-2)), repo_name: task.repo_name, repo_id: String(task.repository_index), revision_snapshot_id: task.successful_land.revision_snapshot_id,
    governance_authority: true, invalidated: false, withdrawn: false, publish_state: "published", evaluation_state: "pass", ci: { overall_status: "pass" } }]));
  const dataFile = path.join(dataRoot, "source-authority.json"); save(dataFile, { nativeHead, nativeLocal, remote, patchsets, roots,
    externals: [...declarations.map(row => ({ ...row, parent_path: "" })), ...nested].map(row => ({ ...row, state: "materialized", dirty: false, linked: false, lock_drift: false })), indices: ids });
  const releaseAit = path.join(dataRoot, "ait-authority"); put(releaseAit, `#!${process.execPath}\nconst fs=require('fs'),a=process.argv.slice(2),d=JSON.parse(fs.readFileSync(${JSON.stringify(dataFile)}));const emit=v=>console.log(JSON.stringify(v));if(a[0]==='status')emit({ok:true,repo_name:'ait-native',line_name:'main',head_snapshot_id:d.nativeHead,worktree:null,workspace:{dirty:false,changed_count:0}});else if(a[0]==='external')emit({command:'external status',externals:d.externals,lock_drifts:[]});else if(a[0]==='config'){const id=Object.keys(d.roots).find(k=>d.roots[k]===process.cwd());emit({repo_name:id==='native'?'ait-native':'ait-'+id,repository_index:id==='native'?28:d.indices[id]});}else if(a[0]==='patchset')emit(d.patchsets[a[2]]);else if(a[1]==='list')emit([d.nativeLocal]);else if(a[0]==='task'&&a[1]==='show'){const v=a[2]==='LNAT-0060'?d.nativeLocal:a.includes('--local')?null:d.remote[a[2]];if(v)emit(v);else{console.error('Error: Unknown Task: '+a[2]);process.exit(1);}}else process.exit(99);\n`); chmodSync(releaseAit, 0o755);

  const home = path.join(root, "approved-home"), bin = path.join(home, ".cargo/bin"); mkdirSync(bin, { recursive: true });
  const childNode = path.join(bin, "node"); copyFileSync(process.execPath, childNode); chmodSync(childNode, 0o755);
  const libraries = path.resolve(path.dirname(process.execPath), "../lib"); if (process.platform === "darwin" && existsSync(libraries)) symlinkSync(libraries, path.join(home, ".cargo/lib"), "dir");
  const providerFile = path.join(dataRoot, "provider.json"), artifacts = [
    { id: 8101, run: 8001, name: "ait-pre-rc-qualification-" + qCommit, file: qZip },
    { id: 8102, run: 8002, name: "ait-pre-tag-candidate-" + release.id, file: dZip },
    { id: 8103, run: 8002, name: "ait-pre-tag-clean-host-" + release.id, file: eZip }];
  save(providerFile, { runs: { 8001: { id: 8001, head_sha: qCommit, status: "completed", conclusion: "success", run_attempt: 1 },
    8002: { id: 8002, head_sha: commit, status: "completed", conclusion: "success", run_attempt: 1 } }, artifacts: artifacts.map(row => ({ ...row,
      record: { id: row.id, name: row.name, expired: false, digest: "sha256:" + sha(row.file), workflow_run: { id: row.run } } })) });
  const gh = path.join(bin, "gh"); put(gh, `#!${process.execPath}\nconst fs=require('fs'),cp=require('child_process'),a=process.argv.slice(2),d=JSON.parse(fs.readFileSync(${JSON.stringify(providerFile)}));if(a[0]==='auth')process.exit(0);const url=a.find(x=>x.startsWith('repos/')),m=url&&/actions\\/runs\\/(\\d+)(\\/artifacts.*)?$/.exec(url),z=url&&/actions\\/artifacts\\/(\\d+)\\/zip$/.exec(url);if(a[0]==='api'&&m&&d.runs[m[1]])console.log(JSON.stringify(m[2]?{artifacts:d.artifacts.filter(x=>x.run===Number(m[1])).map(x=>x.record)}:d.runs[m[1]]));else if(a[0]==='api'&&z){const row=d.artifacts.find(x=>x.id===Number(z[1]));if(!row)process.exit(64);process.stdout.write(fs.readFileSync(row.file));}else{const r=cp.spawnSync(${JSON.stringify(path.join(protocolBin, "gh"))},a,{stdio:'inherit'});process.exit(r.status??65);}\n`); chmodSync(gh, 0o755);
  const cargo = path.join(bin, "cargo"); put(cargo, `#!${process.execPath}\nconst fs=require('fs'),path=require('path'),a=process.argv.slice(2);if(!a.includes('--no-run'))process.exit(64);const suite=a[a.indexOf('--test')+1],exe=path.join(process.env.CARGO_BUILD_BUILD_DIR||path.join(process.cwd(),'.ait/cargo-build/canonical'),'ait-ci','deps',suite);fs.mkdirSync(path.dirname(exe),{recursive:true});fs.writeFileSync(exe,${JSON.stringify(`#!${process.execPath}\nconst c=require('child_process').spawnSync(process.env['CARGO_BIN_EXE_ait-cli'],['--fixture-installed']);if(c.status!==0)process.exit(c.status??65);const s=process.argv[3];process.stdout.write('running 1 test\\ntest '+s+' ... ok\\ntest result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 15 filtered out; finished in 0.1s\\n');\n`)});fs.chmodSync(exe,0o755);console.log(JSON.stringify({reason:'compiler-artifact',target:{name:suite},profile:{test:true},executable:exe}));\n`); chmodSync(cargo, 0o755);
  const env = { ...process.env, HOME: home, PATH: [bin, protocolBin, process.env.PATH].join(path.delimiter), AIT_RELEASE_AIT_BIN: releaseAit,
    AIT_CONDUCTOR_FIXTURE_STATE: protocolState, AIT_RELEASE_OPERATION_ROOT: "", AIT_RELEASE_OPERATION_TOKEN: "" };
  const invoke = (script, args) => spawnSync(childNode, [path.join(nativeCi, script), ...args], { encoding: "utf8", env });
  const authority = spawn(process.execPath, ["--input-type=module", "-e", "import http from 'node:http';const s=http.createServer((q,r)=>{r.setHeader('Content-Type','application/json');r.end(JSON.stringify({repository:{repository_name:'ait-core'}}));});s.listen(0,'127.0.0.1',()=>console.log(s.address().port));"], { stdio: ["ignore", "pipe", "inherit"] });
  try {
    const port = await new Promise((resolve, reject) => { authority.stdout.once("data", bytes => resolve(Number(String(bytes).trim()))); authority.once("error", reject); });
    save(path.join(roots.core, ".ait/config.json"), { default_remote: "origin", remotes: { origin: { url: `http://127.0.0.1:${port}` } } });
    const requestFile = path.join(dataRoot, "request.json"); save(requestFile, { contract: "ait.release.request/v1", release: { version: "1.2.0", prior_version: "1.1.4", channel: "stable" },
      records_root: records, roots, paths: { public_source: source, web_test_root: repository }, web: { server_url: "http://127.0.0.1:8088/", server_data: root,
        seed: 20261029, acceptance_snapshot: correctedCohort.web_test }, github_repository: "owner/repository", winget_fork: "owner/winget", adopt: Object.fromEntries(Object.entries(tasks).map(([id, task]) =>
          [id, { task, snapshot: correctedCohort[id], stage: "snapshot_ready", edit_root: path.join(roots[id], "removed-task-root") }])) });
    const prepared = invoke("release_stable_prepare.mjs", ["run", "--request", requestFile]); assert.equal(prepared.status, 0, prepared.stderr);
    const acceptedFile = path.join(records, "accepted-input.json"), accepted = read(acceptedFile);
    assert.deepEqual(read(accepted.paths.web_components), correctedCohort); assert(!read(path.join(records, "preparation-state.json")).steps.refresh_server);
    const recipe = structuredClone(recoveryRecipe), copy = path.join(root, "fixture-producer.mjs");
    recipe.phases.find(row => row.id === "candidate_freeze").actions = [{ id: "freeze-fixture-candidate", type: "command", mutation: false, cwd: root,
      argv: [process.execPath, copy, candidateFile, "{{records_root}}/frozen-fixture.json"], outputs: ["{{records_root}}/frozen-fixture.json"] }];
    for (const phase of recipe.phases) if (["public_qualification", "component_receipts", "clean_host_qualification"].includes(phase.id)) {
      const first = phase.actions[0], namespace = phase.id + "-approved-fixture"; first.workflow = namespace; first.dispatch_argv[2] = namespace;
      if (phase.id === "public_qualification") phase.actions.push({ id: "qualify-public-source", type: "command", mutation: false, cwd: root,
        argv: [process.execPath, copy, qFile, "{{records_root}}/pre-rc-qualification.json"], outputs: ["{{records_root}}/pre-rc-qualification.json"] });
      if (phase.id === "clean_host_qualification") phase.actions.push({ id: "qualify-frozen-candidate", type: "command", mutation: false, cwd: root,
        argv: [process.execPath, copy, candidateFile, "{{records_root}}/candidate.json"], outputs: ["{{records_root}}/candidate.json"] },
      { id: "cache-frozen-candidate", type: "command", mutation: false, cwd: native, argv: [path.join(nativeCi, "release_artifact_cache.sh"), "owner/repository", "8002", "ait-pre-tag-candidate-", "{{records_root}}/candidate-cache"], outputs: ["{{records_root}}/candidate-cache"] },
      { id: "materialize-host-candidate", type: "command", mutation: true, cwd: native,
        argv: [path.join(nativeCi, "release_candidate_materialize.sh"), "materialize", "1.2.0", "{{records_root}}/candidate-cache", "{{records_root}}/candidate-bin"],
        probe: { cwd: native, argv: [path.join(nativeCi, "release_candidate_materialize.sh"), "probe", "1.2.0", "{{records_root}}/candidate-cache", "{{records_root}}/candidate-bin"] }, outputs: ["{{records_root}}/candidate-bin"] });
    }
    const recipeFile = path.join(dataRoot, "recipe.json"), specFile = path.join(records, "release-spec.json"), planFile = path.join(records, "conductor-plan.json"), stateFile = path.join(records, "conductor-state.json"); save(recipeFile, recipe);
    const spec = invoke("release_stable_spec.mjs", ["--input", acceptedFile, "--output", specFile]); assert.equal(spec.status, 0, spec.stderr);
    const init = () => invoke("release_conductor_init.mjs", ["--spec", specFile, "--recipe", recipeFile, "--plan", planFile]);
    const corrupted = structuredClone(diffs); corrupted[previousCohort.core + ":" + correctedCohort.core].files.push({ path: "rust/crates/ait-core/src/unapproved.rs", status: "modified" }); save(diffFile, corrupted);
    const rejected = init(); assert.equal(rejected.status, 75, rejected.stderr); assert.match(rejected.stderr, /unapproved core source delta/); assert(!existsSync(planFile)); save(diffFile, diffs);
    const initialized = init(); assert.equal(initialized.status, 0, initialized.stderr);
    const web = read(planFile).phases.find(row => row.id === "web_admission").actions; assert.equal(web.length, 7);
    assert(!web[1].adopt && !web[1].probe && !web[5].adopt && !web[5].probe, "fresh cases cannot adopt orphan old captures");
    const initial = web[1].outputs[0]; cpSync(retainedInit, initial, { recursive: true });
    const conduct = () => invoke("release_conductor.mjs", ["run", "--plan", planFile, "--state", stateFile]);
    const orphan = conduct(); assert.equal(orphan.status, 75, orphan.stderr); assert.match(orphan.stderr, /bounded WT batch incomplete/);
    assert.match(readFileSync(path.join(records, "conductor-logs/bounded-web-1.log"), "utf8"), /unbound pre-existing output/);
    renameSync(initial, path.join(dataRoot, "rejected-old-init"));
    const completed = conduct();
    const diagnostics = ["bounded-web-1", "bounded-web-2", "bounded-web-3", "bounded-web-4", "bounded-web-5", "run-web-admission"].map(id => {
      const file = path.join(records, "conductor-logs", id + ".log"); return existsSync(file) ? id + ": " + readFileSync(file, "utf8") : "";
    }).join("\n");
    assert.equal(completed.status, 75, completed.stderr + diagnostics); assert.match(completed.stderr, /Pre-tag operator HOLD/, diagnostics);
    const admission = path.join(records, "web-admission/web-admission.json"), input = path.join(records, "web-admission/qualification-input.json");
    assert.equal(read(admission).status, "admitted"); assert.deepEqual(read(input).components, correctedCohort);
    assert.equal(readFileSync(path.join(root, "installed-binding-dispatch"), "utf8").trim().split("\n").length, 16);
    assert.equal(readFileSync(path.join(root, "fork-dispatch"), "utf8"), "fork\n"); assert.equal(readFileSync(path.join(root, "random-dispatch"), "utf8"), "run\n");
    const verify = () => invoke("release_web_admission_verify.mjs", [input, admission, repository]); assert.equal(verify().status, 0);
    for (const file of [path.join(initial, "binding.005.stdout"), path.join(cacheDirectory(records, "candidate"), "run.json")]) {
      const bytes = readFileSync(file); put(file, "{}\n"); assert.notEqual(verify().status, 0); put(file, bytes);
    }
    assert(!existsSync(path.join(collection, "summary.json")), "original diagnostics must remain unsummarized");
  } finally { authority.kill(); await new Promise(resolve => authority.once("exit", resolve)); }
}

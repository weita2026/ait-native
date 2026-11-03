import assert from "node:assert/strict";
import { chmodSync, copyFileSync, existsSync, mkdirSync, mkdtempSync, readFileSync, realpathSync, rmSync, writeFileSync } from "node:fs";
import { spawn, spawnSync } from "node:child_process";
import { tmpdir } from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { validateRetainedR4Runner, requireDiagnosticR4Results } from "../release/monorepo/ci/source-quality-r4-reuse.mjs";

// Exercise the actual shell entrypoint with tiny owned suites. This checks
// scheduling and failure collection, without rerunning release qualification.
const here = path.dirname(fileURLToPath(import.meta.url));
const root = realpathSync(mkdtempSync(path.join(tmpdir(), "ait-ci-execution-")));
const put = (file, bytes) => { mkdirSync(path.dirname(file), { recursive: true }); writeFileSync(file, bytes); };
const quote = value => "'" + value.replaceAll("'", "'\\''") + "'";
try {
  for (const fail of [false, true]) {
    const repo = path.join(root, fail ? "failure" : "success"), scratch = path.join(repo, "scratch");
    mkdirSync(scratch, { recursive: true }); mkdirSync(path.join(repo, "ci"));
    copyFileSync(path.join(here, "run.sh"), path.join(repo, "ci/run.sh"));
    put(path.join(repo, "ait-release-family.json"), '{"family":{"version":"1.2.0"}}\n');
    put(path.join(repo, "ci/release_family_bind.mjs"), "import fs from 'node:fs';const a=process.argv;fs.writeFileSync(a[a.indexOf('--output')+1],'{}');\n");
    const worker = path.join(repo, "fixture.mjs"), events = path.join(repo, "events.jsonl");
    put(worker, `import fs from 'node:fs';import path from 'node:path';import assert from 'node:assert/strict';
const label=process.argv[2],root=${JSON.stringify(repo)},scratch=fs.realpathSync(process.env.TMPDIR);
assert(scratch.startsWith(root+'/scratch/ait-native-gate.'));
assert(!fs.existsSync(path.join(scratch,'same-name')));fs.writeFileSync(path.join(scratch,'same-name'),label);
const event=phase=>fs.appendFileSync(${JSON.stringify(events)},JSON.stringify({label,phase,scratch})+'\\n');
event('start');
const wait=async file=>{const end=Date.now()+5000;while(!fs.existsSync(root+'/'+file)){assert(Date.now()<end,'fixture peer did not run');await new Promise(r=>setTimeout(r,10));}};
if(label==='composition'){fs.writeFileSync(root+'/composition-started','1');await wait('serial-finished');await new Promise(r=>setTimeout(r,80));}
else if(label==='build')await wait('composition-started');
else if(label==='last')fs.writeFileSync(root+'/serial-finished','1');
console.log(label+' stdout');console.error(label+' stderr');event('end');
if(${fail}&&['composition','first'].includes(label))process.exit(label==='first'?7:8);
`);
    put(path.join(repo, "release/monorepo/build-release.test.mjs"), `process.argv[2]='build';await import(${JSON.stringify(worker)});\n`);
    for (const [name, label] of [["release_first_test.sh", "first"], ["release_second_test.sh", "second"]]) {
      const file = path.join(repo, "ci", name); put(file, `#!/usr/bin/env bash\nexec ${quote(process.execPath)} ${quote(worker)} ${quote(label)}\n`); chmodSync(file, 0o755);
    }
    for (const [name, label] of [["release_web_composition_test.mjs", "composition"], ["release_last_test.mjs", "last"]])
      put(path.join(repo, "ci", name), `process.argv[2]=${JSON.stringify(label)};await import(${JSON.stringify(worker)});\n`);
    put(path.join(repo, "ci/release_npm_namespace_supplement_test.sh"), "echo 'historical supplement must not run' >&2\nexit 99\n");
    const child = spawn("bash", [path.join(repo, "ci/run.sh")], { cwd: repo, env: { ...process.env, TMPDIR: scratch },
      detached: true, stdio: ["ignore", "pipe", "pipe"] });
    let stdout = "", stderr = ""; child.stdout.on("data", bytes => { stdout += bytes; }); child.stderr.on("data", bytes => { stderr += bytes; });
    const timer = setTimeout(() => { try { process.kill(-child.pid, "SIGKILL"); } catch {} }, 15_000);
    let code;
    try { code = await new Promise((resolve, reject) => { child.once("error", reject); child.once("close", resolve); }); }
    finally { clearTimeout(timer); }
    assert.equal(code, fail ? 1 : 0, stdout + stderr);
    const rows = readFileSync(events, "utf8").trim().split("\n").map(line => JSON.parse(line));
    const inventory = ["build", "composition", "first", "second", "last"];
    for (const phase of ["start", "end"]) assert.deepEqual(rows.filter(row => row.phase === phase).map(row => row.label).sort(), [...inventory].sort());
    assert.equal(new Set(rows.filter(row => row.phase === "start").map(row => row.scratch)).size, inventory.length);
    let active = 0, maximum = 0;
    for (const row of rows) { active += row.phase === "start" ? 1 : -1; assert(active >= 0 && active <= 2); maximum = Math.max(maximum, active); }
    assert.equal(active, 0); assert.equal(maximum, 2);
    const serial = rows.filter(row => row.label !== "composition").map(row => row.label + ":" + row.phase);
    assert.deepEqual(serial, ["build", "first", "second", "last"].flatMap(label => [label + ":start", label + ":end"]));
    assert.equal(rows.at(-1).label, "composition", "entrypoint must join the background suite");
    for (const row of rows) assert(!existsSync(row.scratch), "gate cleanup left a worker scratch tree");
    for (const label of inventory) {
      assert(stdout.includes(label + " stdout"));
      // node --test includes its child's stderr in the TAP stream.
      assert((label === "build" ? stdout : stderr).includes(label + " stderr"));
    }
    assert.match(stdout, /historical rc\.3 supplement pinned/); assert(!stderr.includes("historical supplement must not run"));
    assert.match(stdout, /complete Native CI: [0-9]+s, exit/);
    if (fail) { assert.match(stderr, /release_first_test\.sh: FAIL \(exit 7/); assert.match(stderr, /release_web_composition_test\.mjs: FAIL \(exit 8/); }
  }
  // Model the relevant Actions step scheduling, using the actual Unix run
  // commands and final gate. Core failure must retain Runner's own result.
  const quality = readFileSync(path.join(here, "../release/monorepo/.github/workflows/ait-source-quality.yml"), "utf8");
  const steps = quality.split(/(?=^      - name:)/m);
  const step = id => {
    const body = steps.find(value => value.includes("        id: " + id + "\n"));
    assert(body, "missing source-quality component step " + id);
    assert.match(body, /continue-on-error: true/);
    return body;
  };
  for (const id of ["core_unix", "runner_unix", "core_windows", "runner_windows"]) step(id);
  const command = id => /        run: ([^\n]+)/.exec(step(id))[1];
  const gate = steps.find(value => value.includes("Require both component results"));
  assert.match(gate, /if: always\(\)/);
  assert.match(gate, /steps\.core_windows\.outcome.*steps\.core_unix\.outcome/);
  assert.match(gate, /steps\.runner_windows\.outcome.*steps\.runner_unix\.outcome/);
  const gateBody = /        run: \|\n((?:          .*\n|\n)*)/.exec(gate)[1].split("\n").map(line => line.startsWith("          ") ? line.slice(10) : line).join("\n");
  const qualityRoot = path.join(root, "quality"); mkdirSync(qualityRoot);
  for (const component of ["core", "runner"]) put(path.join(qualityRoot, "ait-" + component, "ci/run.sh"),
    'printf "' + component + '\\n" >>"$quality_events"\nexit "${quality_' + component + '_exit}"\n');
  for (const [coreExit, runnerExit] of [[0, 0], [7, 0], [0, 9], [7, 9]]) {
    const events = path.join(qualityRoot, "events-" + coreExit + "-" + runnerExit);
    const env = { ...process.env, DIAGNOSTIC_REUSE_R4: "false", quality_events: events, quality_core_exit: String(coreExit), quality_runner_exit: String(runnerExit) };
    const core = spawnSync("/bin/bash", ["-c", command("core_unix")], { cwd: qualityRoot, env });
    const runner = spawnSync("/bin/bash", ["-c", command("runner_unix")], { cwd: qualityRoot, env });
    const result = spawnSync("/bin/bash", ["-c", gateBody], { cwd: qualityRoot, env: { ...env,
      CORE_RESULT: core.status === 0 ? "success" : "failure", RUNNER_RESULT: runner.status === 0 ? "success" : "failure" } });
    assert.deepEqual(readFileSync(events, "utf8").trim().split("\n"), ["core", "runner"]);
    assert.equal(result.status === 0, coreExit === 0 && runnerExit === 0);
  }
  for (const result of ["skipped", "cancelled", ""]) {
    assert.notEqual(spawnSync("/bin/bash", ["-c", gateBody], { env: { ...process.env, DIAGNOSTIC_REUSE_R4: "false", CORE_RESULT: result, RUNNER_RESULT: "success" } }).status, 0);
  }

  // Retained evidence fixtures exercise the real verifier, not qualification.
  // The Mac job failed Core while its Runner outcome passed. A normalized
  // REST step conclusion alone must never turn a failed Runner into reuse.
  const r4Repository = "weita2026/ait-native", r4Head = "c1f64918e6f2360f2029696f554e32760acc310d";
  const r4Branch = "diagnostic/release-121-20261103-r4", r4RunId = 41544513498;
  const r4Run = { id: r4RunId, run_attempt: 1, head_sha: r4Head, head_branch: r4Branch,
    path: ".github/workflows/ait-source-quality.yml", name: "AIT source quality", event: "workflow_dispatch",
    status: "completed", conclusion: "failure", repository: { full_name: r4Repository }, head_repository: { full_name: r4Repository } };
  const r4Job = os => ({ id: os === "Linux" ? 125616699768 : 125616700080, run_id: r4RunId, run_attempt: 1,
    head_sha: r4Head, head_branch: r4Branch, workflow_name: r4Run.name, name: `Locked Core and Runner / ${os === "Linux" ? "ubuntu-22.04" : "macos-14"}`,
    status: "completed", conclusion: os === "Linux" ? "success" : "failure", labels: [os === "Linux" ? "ubuntu-22.04" : "macos-14"],
    steps: [{ name: "Runner gates (Unix)", status: "completed", conclusion: "success" }] });
  const r4Log = os => `2026-11-02T23:28:12.0000000Z Core: ${os === "Linux" ? "success" : "failure"}\n2026-11-02T23:28:12.0000000Z Runner: success\n`;
  for (const os of ["Linux", "macOS"]) {
    assert.match(validateRetainedR4Runner(r4Run, r4Job(os), r4Log(os), os), /Runner: reused R4.*not executed/);
    for (const mutate of [
      run => { run.id++; }, run => { run.run_attempt++; }, run => { run.head_sha = "0".repeat(40); },
      run => { run.head_branch = "main"; }, run => { run.path = "another.yml"; }, run => { run.name = "another"; },
      run => { run.event = "push"; }, run => { run.status = "in_progress"; }, run => { run.conclusion = "success"; },
      run => { run.repository.full_name = "another/repo"; }, run => { delete run.head_repository; },
    ]) { const run = structuredClone(r4Run); mutate(run); assert.throws(() => validateRetainedR4Runner(run, r4Job(os), r4Log(os), os)); }
    for (const mutate of [
      job => { job.id++; }, job => { job.run_id++; }, job => { job.run_attempt++; }, job => { job.head_sha = "0".repeat(40); },
      job => { job.head_branch = "main"; }, job => { job.workflow_name = "another"; }, job => { job.name = "another"; },
      job => { job.labels = []; }, job => { job.status = "in_progress"; }, job => { job.conclusion = "cancelled"; },
      job => { job.steps = []; }, job => { job.steps.push(job.steps[0]); },
      job => { job.steps[0].status = "in_progress"; }, job => { job.steps[0].conclusion = "failure"; },
      job => { job.steps[0].conclusion = "skipped"; },
    ]) { const job = r4Job(os); mutate(job); assert.throws(() => validateRetainedR4Runner(r4Run, job, r4Log(os), os)); }
    for (const logs of ["", r4Log(os).replace("Runner: success", "Runner: failure"), r4Log(os) + "Runner: success\n",
      r4Log(os).replace(`Core: ${os === "Linux" ? "success" : "failure"}`, "Core: cancelled")]) {
      assert.throws(() => validateRetainedR4Runner(r4Run, r4Job(os), logs, os));
    }
    const context = { event: "workflow_dispatch", repository: r4Repository, ref: "refs/heads/diagnostic/release-121-fixture", os, core: "success", runner: "skipped" };
    const calls = [], api = { json: async resource => { calls.push(resource); return resource.includes("/runs/") ? r4Run : r4Job(os); },
      text: async resource => { calls.push(resource); return r4Log(os); } };
    const lines = await requireDiagnosticR4Results(context, api); assert(lines.some(line => line.startsWith("Runner: reused R4")));
    assert.deepEqual(calls, [`/repos/${r4Repository}/actions/runs/${r4RunId}/attempts/1`,
      `/repos/${r4Repository}/actions/jobs/${r4Job(os).id}`, `/repos/${r4Repository}/actions/jobs/${r4Job(os).id}/logs`]);
    calls.length = 0;
    await assert.rejects(requireDiagnosticR4Results({ ...context, core: "failure" }, api), /fresh Core gate/);
    assert.equal(calls.length, 3, "fresh failure must still collect independent retained evidence");
    for (const runner of ["success", "failure", "cancelled", ""]) await assert.rejects(requireDiagnosticR4Results({ ...context, runner }, api), /must not execute/);
    await assert.rejects(requireDiagnosticR4Results(context, { ...api, json: async () => { throw new Error("evidence unavailable"); } }), /evidence unavailable/);
    for (const change of [{ event: "push" }, { repository: "another/repo" }, { ref: "refs/heads/main" }, { ref: `refs/heads/${r4Branch}` }, { os: "unknown" }]) {
      await assert.rejects(requireDiagnosticR4Results({ ...context, ...change }, api));
    }
  }
  const windowsContext = { event: "workflow_dispatch", repository: r4Repository, ref: "refs/heads/diagnostic/release-121-fixture", os: "Windows", core: "success", runner: "success" };
  const noReuse = { json: () => { throw new Error("Windows must not reuse R4"); }, text: () => { throw new Error("Windows must not reuse R4"); } };
  assert((await requireDiagnosticR4Results(windowsContext, noReuse)).includes("Runner: fresh success"));
  await assert.rejects(requireDiagnosticR4Results({ ...windowsContext, runner: "skipped" }, noReuse), /fresh Windows Runner/);
  await assert.rejects(requireDiagnosticR4Results({ ...windowsContext, core: "failure", runner: "failure" }, noReuse),
    error => error.message.includes("fresh Core gate") && error.message.includes("fresh Windows Runner"));
  assert.match(step("runner_unix"), /!\(github.event_name == 'workflow_dispatch' && inputs.diagnostic_reuse_r4_unix_runner\)/);
  assert.match(quality, /diagnostic_reuse_r4_unix_runner:[\s\S]*?type: boolean[\s\S]*?default: false/);
  assert.match(quality, /fail-fast: false/); assert.match(gate, /steps\.runner_windows\.outcome/);
  mkdirSync(path.join(qualityRoot, "ci"));
  copyFileSync(path.join(here, "../release/monorepo/ci/source-quality-r4-reuse.mjs"), path.join(qualityRoot, "ci/source-quality-r4-reuse.mjs"));
  for (const [core, runner, success] of [["success", "success", true], ["failure", "success", false], ["success", "failure", false], ["success", "skipped", false]]) {
    const result = spawnSync("/bin/bash", ["-c", gateBody], { cwd: qualityRoot, encoding: "utf8", env: { ...process.env,
      DIAGNOSTIC_REUSE_R4: "true", SOURCE_QUALITY_EVENT: "workflow_dispatch", SOURCE_QUALITY_REPOSITORY: r4Repository,
      SOURCE_QUALITY_REF: windowsContext.ref, RUNNER_OS: "Windows", CORE_RESULT: core, RUNNER_RESULT: runner } });
    assert.equal(result.status === 0, success, result.stdout + result.stderr);
    if (success) assert.match(result.stdout, /Runner: fresh success/);
  }

  // Execute the diagnostic PowerShell collector with fake Cargo commands.
  // This proves scheduling/isolation only, never a native Windows result.
  const probeJob = quality.split("  windows-diagnostic:\n")[1]; assert(probeJob);
  assert.match(probeJob, /runs-on: windows-2025/);
  assert.match(probeJob, /github.event_name == 'workflow_dispatch' && \(inputs.diagnostic_windows_batch \|\| inputs.diagnostic_runner_windows_verify \|\| inputs.diagnostic_affected_windows_verify\)/);
  assert(!/ait-core\/ci\/run|ait-runner\/ci\/run/.test(probeJob));
  assert.match(quality.split("  locked-rust:\n")[1].split("    strategy:")[0], /!\(github.event_name == 'workflow_dispatch' && \(inputs.diagnostic_windows_batch \|\| inputs.diagnostic_runner_windows_verify \|\| inputs.diagnostic_affected_windows_verify\)\)/);
  assert.match(probeJob, /if: always\(\) && \(inputs.diagnostic_windows_batch \|\| inputs.diagnostic_affected_windows_verify\) && steps\.diagnostic\.outputs\.evidence_root/);
  assert.match(probeJob, /node -p.*rust_toolchain/); assert.match(probeJob, /exit \$LASTEXITCODE/);
  const pwsh = process.env.AIT_CI_TEST_POWERSHELL ?? "pwsh";
  const available = spawnSync(pwsh, ["-NoProfile", "-Command", "$PSVersionTable.PSVersion.ToString()"], { encoding: "utf8" });
  if (available.status === 0) {
    const psQuote = value => "'" + value.replaceAll("'", "''") + "'";
    for (const [coreExit, testExit, lintExit] of [0, 1].flatMap(core => [[core, 0, 0], [core, 101, 0], [core, 0, 101], [core, 101, 103]])) {
      const fixture = path.join(root, `runner-probe-${coreExit}-${testExit}-${lintExit}`), events = path.join(fixture, "cargo-events.jsonl"), attempt = path.join(fixture, "attempt");
      put(path.join(fixture, "ait-runner/src/server.rs"), "offline transport fixture, not the frozen source\n");
      const probe = path.join(fixture, "ci/windows-diagnostic.ps1"); mkdirSync(path.dirname(probe), { recursive: true });
      copyFileSync(path.join(here, "../release/monorepo/ci/windows-diagnostic.ps1"), probe);
      put(path.join(fixture, "ci/core-windows-probe.ps1"), `$env:CARGO_TARGET_DIR='must-not-leak';$env:TMPDIR='must-not-leak';exit ${coreExit}\n`);
      const cargo = path.join(fixture, "cargo.mjs");
      put(cargo, `import fs from 'node:fs';fs.appendFileSync(${JSON.stringify(events)},JSON.stringify({args:process.argv.slice(2),cwd:process.cwd(),tmp:process.env.TMPDIR,home:process.env.CARGO_HOME,target:process.env.CARGO_TARGET_DIR,build:process.env.CARGO_BUILD_BUILD_DIR,incremental:process.env.CARGO_INCREMENTAL})+'\\n');process.exit(process.argv[2]==='test'?${testExit}:${lintExit});\n`);
      const launcher = path.join(fixture, "launch.ps1");
      put(launcher, `function Get-FileHash { param($LiteralPath,$Algorithm) [pscustomobject]@{Hash='de7e870c2f622c23fb6514d5f88f20d8b71c9d0b6a57e4730605e997be6e8fb5'} }\nfunction cargo { & ${psQuote(process.execPath)} ${psQuote(cargo)} @args; $global:LASTEXITCODE=$LASTEXITCODE }\n& ${psQuote(probe)}\nexit $LASTEXITCODE\n`);
      const env = { ...process.env, RUNNER_OS: "Windows", SOURCE_QUALITY_EVENT: "workflow_dispatch", SOURCE_QUALITY_REPOSITORY: r4Repository,
        SOURCE_QUALITY_REF: "refs/heads/diagnostic/release-121-fixture", DIAGNOSTIC_REUSE_R4: "false", DIAGNOSTIC_WINDOWS_BATCH: "true", DIAGNOSTIC_RUNNER_WINDOWS_VERIFY: "false", DIAGNOSTIC_AFFECTED_WINDOWS_VERIFY: "false",
        AIT_RUNNER_ATTEMPT_ROOT: attempt, RUNNER_TEMP: path.join(fixture, "evidence"), GITHUB_OUTPUT: "" };
      const result = spawnSync(pwsh, ["-NoProfile", "-File", launcher], { cwd: fixture, env, encoding: "utf8", timeout: 20_000 });
      assert.equal(result.status, coreExit || testExit || lintExit ? 1 : 0, result.stdout + result.stderr);
      const rows = readFileSync(events, "utf8").trim().split("\n").map(JSON.parse);
      assert.deepEqual(rows.map(row => row.args), [
        ["test", "--locked", "--lib", "server::tests::legacy_transition_executes_and_preserves_attempt_cleanup", "--", "--exact", "--nocapture"],
        ["clippy", "--locked", "--workspace", "--all-targets", "--", "-D", "warnings"],
      ]);
      for (const row of rows) {
        assert.equal(realpathSync(row.cwd), realpathSync(path.join(fixture, "ait-runner")));
        for (const name of ["tmp", "home", "target", "build"]) assert(row[name].startsWith(attempt + path.sep + "repository-ci" + path.sep));
        assert.equal(row.incremental, "0");
      }
      assert(!existsSync(path.join(attempt, "repository-ci")), "probe scratch must be cleaned after every result");
      assert.match(result.stdout, /CI RUN core-selected-diagnostics/); assert.match(result.stdout, /CI RUN runner-strict-clippy/);
      for (const change of [{ RUNNER_OS: "Linux" }, { SOURCE_QUALITY_EVENT: "push" }, { SOURCE_QUALITY_REPOSITORY: "other/repo" },
        { SOURCE_QUALITY_REF: "refs/heads/main" }, { SOURCE_QUALITY_REF: "refs/heads/diagnostic/release-121-20261103-r5" },
        { SOURCE_QUALITY_REF: "refs/heads/diagnostic/release-121-20261103-r6" }, { DIAGNOSTIC_REUSE_R4: "true" },
        { DIAGNOSTIC_WINDOWS_BATCH: "false" }, { DIAGNOSTIC_RUNNER_WINDOWS_VERIFY: "true" }, { DIAGNOSTIC_RUNNER_WINDOWS_VERIFY: "unknown" },
        { DIAGNOSTIC_AFFECTED_WINDOWS_VERIFY: "true" }, { DIAGNOSTIC_AFFECTED_WINDOWS_VERIFY: "unknown" }]) {
        const before = readFileSync(events); const rejected = spawnSync(pwsh, ["-NoProfile", "-File", launcher], { cwd: fixture, env: { ...env, ...change }, encoding: "utf8", timeout: 20_000 });
        assert.notEqual(rejected.status, 0); assert.deepEqual(readFileSync(events), before);
      }
      put(launcher, readFileSync(launcher, "utf8").replace("de7e870c2f622c23fb6514d5f88f20d8b71c9d0b6a57e4730605e997be6e8fb5", "0".repeat(64)));
      const before = readFileSync(events), wrongHash = spawnSync(pwsh, ["-NoProfile", "-File", launcher], { cwd: fixture, env, encoding: "utf8", timeout: 20_000 });
      assert.notEqual(wrongHash.status, 0); assert.deepEqual(readFileSync(events), before);
    }
    for (const [testExit, affected, coreExit] of [[0, false, 0], [101, false, 0], ...[0, 101].flatMap(test => [0, 1].map(core => [test, true, core]))]) {
      const fixture = path.join(root, `runner-verify-${testExit}-${affected}-${coreExit}`), events = path.join(fixture, "events.jsonl"), attempt = path.join(fixture, "attempt"), coreMarker = path.join(fixture, "core-selected");
      put(path.join(fixture, "ait-runner/src/server.rs"), "offline control fixture\n");
      const probe = path.join(fixture, "ci/windows-diagnostic.ps1"); mkdirSync(path.dirname(probe), { recursive: true });
      copyFileSync(path.join(here, "../release/monorepo/ci/windows-diagnostic.ps1"), probe);
      put(path.join(fixture, "ci/core-windows-probe.ps1"), affected
        ? `[IO.File]::WriteAllText(${psQuote(coreMarker)},$env:DIAGNOSTIC_AFFECTED_WINDOWS_VERIFY);$env:CARGO_TARGET_DIR='must-not-leak';exit ${coreExit}\n`
        : "throw 'Core must not execute during Runner verification'\n");
      const cargo = path.join(fixture, "cargo.mjs");
      put(cargo, `import fs from 'node:fs';const args=process.argv.slice(2);fs.appendFileSync(${JSON.stringify(events)},JSON.stringify(args)+'\\n');process.exit(args[0]==='test'?${testExit}:99);\n`);
      const launcher = path.join(fixture, "launch.ps1");
      put(launcher, `function Get-FileHash { param($LiteralPath,$Algorithm) [pscustomobject]@{Hash='3798355c96d796edeacbaea4e661df9a2f9ab02b645a33edc874328141635e02'} }\nfunction cargo { & ${psQuote(process.execPath)} ${psQuote(cargo)} @args; $global:LASTEXITCODE=$LASTEXITCODE }\n& ${psQuote(probe)}\nexit $LASTEXITCODE\n`);
      const env = { ...process.env, RUNNER_OS: "Windows", SOURCE_QUALITY_EVENT: "workflow_dispatch", SOURCE_QUALITY_REPOSITORY: r4Repository,
        SOURCE_QUALITY_REF: "refs/heads/diagnostic/release-121-fixture", DIAGNOSTIC_REUSE_R4: "false", DIAGNOSTIC_WINDOWS_BATCH: "false", DIAGNOSTIC_RUNNER_WINDOWS_VERIFY: String(!affected), DIAGNOSTIC_AFFECTED_WINDOWS_VERIFY: String(affected),
        AIT_RUNNER_ATTEMPT_ROOT: attempt, RUNNER_TEMP: path.join(fixture, "evidence"), GITHUB_OUTPUT: "" };
      const result = spawnSync(pwsh, ["-NoProfile", "-File", launcher], { cwd: fixture, env, encoding: "utf8", timeout: 20_000 });
      assert.equal(result.status, testExit || coreExit ? 1 : 0, result.stdout + result.stderr);
      assert.deepEqual(readFileSync(events, "utf8").trim().split("\n").map(JSON.parse), [[
        "test", "--locked", "--lib", "server::tests::legacy_transition_executes_and_preserves_attempt_cleanup", "--", "--exact", "--nocapture",
      ]]);
      assert.equal(result.stdout.includes("CI RUN core-selected-diagnostics"), affected); assert(!result.stdout.includes("CI RUN runner-strict-clippy"));
      if (affected) assert.equal(readFileSync(coreMarker, "utf8"), "true");
      else assert.match(result.stdout, /Core: not selected/);
      assert.match(result.stdout, /Runner Clippy: not executed/);
      assert(!existsSync(path.join(attempt, "repository-ci")));
      put(launcher, readFileSync(launcher, "utf8").replace("3798355c96d796edeacbaea4e661df9a2f9ab02b645a33edc874328141635e02", "de7e870c2f622c23fb6514d5f88f20d8b71c9d0b6a57e4730605e997be6e8fb5"));
      const before = readFileSync(events), stale = spawnSync(pwsh, ["-NoProfile", "-File", launcher], { cwd: fixture, env, encoding: "utf8", timeout: 20_000 });
      assert.notEqual(stale.status, 0); assert.deepEqual(readFileSync(events), before);
    }
    const coreFixtureModes = ["success", "build-failure", "instrumentation-failure", "case-failure", "trace-failure", "nonshared-failure", "foreign-cli"].map(mode => [mode, false]);
    coreFixtureModes.push(...["success", "shared-libraries-failure", "shared-integrations-failure", "core87-guard-failure", "nonshared-failure", "migration-failure", "wrong-source"].map(mode => [mode, true]));
    for (const [mode, affected] of coreFixtureModes) {
      const fixture = path.join(root, `core-probe-${mode}-${affected}`), core = path.join(fixture, "ait-core"), owned = path.join(fixture, "owned"), evidence = path.join(fixture, "evidence"), events = path.join(fixture, "core-events.jsonl");
      put(path.join(core, "rust/crates/ait-cli/Cargo.toml"), "offline control fixture\n");
      const script = path.join(fixture, "ci/core-windows-probe.ps1"); mkdirSync(path.dirname(script), { recursive: true });
      copyFileSync(path.join(here, "../release/monorepo/ci/core-windows-probe.ps1"), script);
      const launcher = path.join(fixture, "core-fixture.ps1");
      put(launcher, `. ${psQuote(script)}
$fixtureMode=${psQuote(mode)}
$fixtureOwned=${psQuote(owned)}
$fixtureManifest=${psQuote(path.join(core, "rust/crates/ait-cli/Cargo.toml"))}
$fixtureCore=${psQuote(core)}
$fixturePins=@{
 'rust/crates/ait-benchmark/src/runner.rs'='fd61ef692212af06b58d53c39457a8a697067071936e65f772d512e053569488'
 'rust/crates/ait-cli/src/primitives/git_interop/native/migration.rs'='012992672677429bfb6bf7e80b130cdee6a6e440378b9e144b553310f5d2d7c7'
 'rust/crates/ait-cli/src/primitives/workflow/bound_worktree.rs'='765c51c23d0f36b0557925e699da749887a6d1c5b24ddadf7847209c13382bfe'
 'rust/crates/ait-cli/src/primitives/worktree.rs'='dfdcc6b816eafd10b786a4757eb020a032a7e5c14deee75da5ec729c054c889b'
 'rust/crates/ait-cli/src/primitives/worktree/cleanup.rs'='978b16d120499fafab017350412dc271b1fe2c25d921079aac67762b0a589f2e'
}
function Get-FileHash($LiteralPath,$Algorithm) {
 $relative=[IO.Path]::GetRelativePath($fixtureCore,$LiteralPath).Replace('\\','/')
 $hash=if($fixtureMode -eq 'wrong-source'){'0'*64}else{$fixturePins[$relative]}
 return [pscustomobject]@{Hash=$hash}
}
function Add-FixtureEvent($Name,$Arguments,$Directory) {
  [IO.File]::AppendAllText(${psQuote(events)},(@{name=$Name;arguments=@($Arguments);directory=$Directory}|ConvertTo-Json -Compress)+[Environment]::NewLine)
}
function Start-CoreCapture($Evidence) { Add-FixtureEvent 'capture-start' @() ''; if($fixtureMode -eq 'instrumentation-failure'){throw 'fixture driver unavailable'};return [pscustomobject]@{Evidence=$Evidence} }
function Stop-CoreCapture($Session) { Add-FixtureEvent 'capture-stop' @() '';if($fixtureMode -eq 'trace-failure'){throw 'fixture PML unavailable'} }
function Invoke-CoreProcess($Program,$Arguments,$Directory,$Stem,$TimeoutMs) {
  $name=[IO.Path]::GetFileName($Stem)
  Add-FixtureEvent $name $Arguments $Directory
  if($name -eq 'cleanup-build') {
    if($fixtureMode -eq 'build-failure'){return [pscustomobject]@{Code=101;Stdout='';Stderr='fixture build failure'}}
    $exe=Join-Path $fixtureOwned 'cargo-target/test.exe';$cli=Join-Path $fixtureOwned 'cargo-target/ait-cli.exe'
    if($fixtureMode -eq 'foreign-cli'){$cli=${psQuote(path.join(fixture, "foreign-cli.exe"))}}
    [IO.File]::WriteAllText($exe,'offline fixture');[IO.File]::WriteAllText($cli,'offline fixture')
    $records=@(
      @{reason='compiler-artifact';target=@{name='workflow_friction_cli';kind=@('test')};profile=@{test=$true};manifest_path=$fixtureManifest;executable=$exe},
      @{reason='compiler-artifact';target=@{name='ait-cli';kind=@('bin')};profile=@{test=$false};manifest_path=$fixtureManifest;executable=$cli})
    $json=($records|ForEach-Object{ConvertTo-Json -InputObject $_ -Depth 8 -Compress}) -join [Environment]::NewLine
    return [pscustomobject]@{Code=0;Stdout=$json;Stderr=''}
  }
  $code=if(($fixtureMode -eq 'case-failure' -and $name -eq 'cleanup-case') -or ($fixtureMode -eq 'nonshared-failure' -and $name -eq 'nonshared-benchmark') -or ($fixtureMode -eq 'migration-failure' -and $name -eq 'nonshared-cli') -or $fixtureMode -eq ($name+'-failure')){101}else{0}
  return [pscustomobject]@{Code=$code;Stdout='offline fixture';Stderr=''}
}
$failed=Invoke-CoreDiagnostics ${psQuote(core)} ${psQuote(owned)} ${psQuote(evidence)} $${affected ? "true" : "false"}
if($failed){exit 1};exit 0
`);
      const result = spawnSync(pwsh, ["-NoProfile", "-File", launcher], { cwd: fixture, encoding: "utf8", timeout: 20_000 });
      assert.equal(result.status, mode === "success" ? 0 : 1, result.stdout + result.stderr);
      if (mode === "wrong-source") { assert(!existsSync(events), "source mismatch must reject before any Cargo or capture"); continue; }
      const rows = readFileSync(events, "utf8").trim().split("\n").map(JSON.parse), names = rows.map(row => row.name);
      const traced = !["build-failure", "instrumentation-failure", "foreign-cli"].includes(mode);
      assert.deepEqual(names, affected ? ["shared-libraries", "shared-integrations", "core87-guard", "nonshared-benchmark", "nonshared-cli"] : ["cleanup-build", ...(["build-failure", "foreign-cli"].includes(mode) ? [] : ["capture-start"]),
        ...(traced ? ["cleanup-case", "capture-stop"] : []), "nonshared-benchmark", "nonshared-cli", "nonshared-leases"]);
      if (!affected) assert.deepEqual(rows[0].arguments, ["test", "--manifest-path", "rust/Cargo.toml", "--profile", "ait-ci", "--locked", "--all-features", "-p", "ait-cli", "--test", "workflow_friction_cli", "--no-run", "--message-format=json"]);
      if (traced && !affected) {
        const selected = rows.find(row => row.name === "cleanup-case");
        assert.deepEqual(selected.arguments, ["finish_closes_nested_checkboxes_under_the_bound_item_only", "--exact", "--nocapture", "--test-threads=1"]);
        assert.equal(realpathSync(selected.directory), realpathSync(path.join(core, "rust/crates/ait-cli")));
      }
      const counts = rows.filter(row => row.name.startsWith("nonshared-")).map(row => {
        for (const flag of ["--locked", "--all-features", "--no-fail-fast", "--lib", "--exact", "--nocapture"]) assert(row.arguments.includes(flag));
        assert.equal(row.arguments[row.arguments.indexOf("--profile") + 1], "ait-ci");
        return row.arguments.slice(row.arguments.indexOf("--") + 1, row.arguments.indexOf("--exact")).length;
      });
      assert.deepEqual(counts, affected ? [2, 6] : [1, 8, 2]);
      if (affected) {
        const common = ["test", "--manifest-path", "rust/Cargo.toml", "--profile", "ait-ci", "--locked", "--all-features", "--no-fail-fast"];
        assert.deepEqual(rows.find(row => row.name === "shared-libraries").arguments,
          [...common, "-p", "ait-cli", "--lib", "--", "primitives::worktree::cleanup", "closeout_context", "--nocapture"]);
        assert.deepEqual(rows.find(row => row.name === "shared-integrations").arguments,
          [...common, "-p", "ait-cli", "--test", "patchset_ci_runner", "--test", "workflow_friction_cli", "--test", "workflow_recovery_cli", "--test", "snapshot_task_binding_cli", "--", "--nocapture"]);
        assert.deepEqual(rows.find(row => row.name === "core87-guard").arguments,
          [...common, "-p", "ait-core", "--lib", "--", "current_source_cache::tests::cli_bootstrap_rejects_changed_content_under_nonadvancing_maximum", "--exact", "--nocapture"]);
        for (const row of rows) {
          assert.equal(realpathSync(row.directory), realpathSync(core));
          assert(!row.arguments.some(arg => arg.startsWith("--test-threads")), "keep the existing default concurrency");
        }
        const benchmark = rows.find(row => row.name === "nonshared-benchmark").arguments;
        assert.deepEqual(benchmark.slice(benchmark.indexOf("--") + 1, benchmark.indexOf("--exact")), [
          "runner::tests::timed_external_subject_records_resource_and_exit_contract",
          "runner::tests::unexpected_exit_is_retained_as_failure_evidence",
        ]);
        const migration = rows.find(row => row.name === "nonshared-cli").arguments;
        const cases = migration.slice(migration.indexOf("--") + 1, migration.indexOf("--exact"));
        assert(cases.every(name => name.startsWith("primitives::git_interop::native::migration::tests::legacy_migration_")));
        assert(!rows.some(row => row.arguments.some(arg => /mirror_resumes|preserve_remote_urls|lease_store_trait_object/.test(arg))));
      }
    }
    console.log("PowerShell diagnostic collectors: independent stages, capture boundaries, exact inventory, guards and cleanup passed (local control fixtures only)");
  } else {
    assert(!process.env.AIT_CI_TEST_POWERSHELL, "the explicitly selected PowerShell control runtime is unavailable");
    console.log("PowerShell diagnostic collector not executed: set AIT_CI_TEST_POWERSHELL to an existing runtime");
  }
} finally { rmSync(root, { recursive: true, force: true }); }
console.log("Native CI complete inventory, bounded lanes, isolation, failure collection and joins passed");

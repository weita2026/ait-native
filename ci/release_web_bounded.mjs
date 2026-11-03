// Private conductor adapter, imported by generated ordinary command actions.
// There is no standalone CLI and no change to the frozen WT acceptance tree.
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { existsSync, mkdirSync, mkdtempSync, cpSync, rmSync, readdirSync, lstatSync, writeFileSync } from "node:fs";
import { spawn, spawnSync } from "node:child_process";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { regular, validateFinalWebState, verifyBoundedCase } from "./release_web_composition.mjs";
import { webAcceptanceTree, validateComposedWebInputs } from "./release_web_controls.mjs";
import { reviewedCompositionLocations } from "./release_web_composition_inputs.mjs";
import { qualificationCarrier, sameProductComponents, correctedWTIdentity, productComponents } from "./release_web_recovery_identity.mjs";
import { recoveryLocations, recoveryEvidence } from "./release_web_recovery_inputs.mjs";
import { isApprovedReplacement, replacementIdentity, previousCohort, qualificationCacheActions } from "./release_web_applicability.mjs";
import { teamCaptureProgram } from "./release_core_team_cases.mjs";

const hash = bytes => createHash("sha256").update(bytes).digest("hex");
const names = ["ait.semantic.init", "native.current.reviewed-fork", "native.current.two-authorities", "randomized_3000", "closeout"];
const here = path.dirname(fileURLToPath(import.meta.url));
const call = (argv, options) => {
  const result = spawnSync(argv[0], argv.slice(1), { encoding: "utf8", maxBuffer: 32 * 1024 * 1024, ...options });
  if (result.status !== 0) {
    process.stdout.write(result.stdout ?? ""); process.stderr.write(result.stderr ?? "");
    const output = argv.includes("--output-dir") ? argv[argv.indexOf("--output-dir") + 1] : options.env.AIT_WEB_TEST_RESULTS_DIR;
    throw new Error(`bounded WT command failed (${result.status ?? result.error}); preserve output at ${output}`);
  }
  return result.stdout;
};

export function validateCompositionSelection(spec, inputPath) {
  const input = JSON.parse(regular(inputPath)), native = spec.values.native_root, repository = spec.values.web_test_root;
  const acceptedBytes = regular(spec.values.accepted_input_path), accepted = JSON.parse(acceptedBytes);
  assert.equal(hash(acceptedBytes), spec.values.accepted_input_sha256, "selected accepted input changed");
  assert.equal(accepted.contract, "ait.release.accepted-input/v1");
  assert.equal(accepted.records_root, spec.records_root); assert.deepEqual(accepted.release, spec.release);
  assert.deepEqual(accepted.component_closeouts, spec.collections.component_closeouts, "selected closeouts differ from accepted input");
  for (const [key, value] of [["native_root", native], ["web_test_root", repository], ["web_components", spec.values.web_components], ["public_source", spec.values.public_source]])
    assert.equal(accepted.paths[key], value, `selected ${key} differs from accepted input`);
  const components = JSON.parse(regular(accepted.paths.web_components));
  const applicability = replacementIdentity(accepted, input, components);
  if (!applicability) sameProductComponents(components, input);
  correctedWTIdentity(accepted, input, applicability ? { ...input.components, web_test: components.web_test } : components);
  assert.deepEqual(accepted.component_closeouts.map(row => row.id).sort(), ["core", "node", "python", "runner", "server"]);
  for (const row of accepted.component_closeouts) assert.equal(row.snapshot, (applicability ? components : input.components)[row.id], `selected ${row.id} source differs from reviewed input`);
  if (applicability) {
    assert.equal(spec.values.github_repository, accepted.github_repository); return applicability;
  }
  const oldPlan = JSON.parse(regular(path.join(path.dirname(path.dirname(inputPath)), "conductor-plan.json")));
  const oldBind = oldPlan.phases.flatMap(phase => phase.actions).filter(action => action.id === "bind-web-admission-input");
  assert.equal(oldBind.length, 1);
  assert.equal(oldBind[0].argv.filter(value => value === "--candidate").length, 1);
  const historical = JSON.parse(regular(oldBind[0].argv[oldBind[0].argv.indexOf("--candidate") + 1]));
  const selectedRoot = qualificationCarrier(accepted) ?? path.dirname(accepted.paths.web_components);
  const selectedCandidate = JSON.parse(regular(path.join(selectedRoot, "candidate.json")));
  assert.deepEqual(selectedCandidate, historical, "selected candidate/artifact/authority differs from reviewed qualification");
  const selectedQualification = JSON.parse(regular(path.join(selectedRoot, "pre-rc-qualification.json")));
  assert.deepEqual(selectedQualification, JSON.parse(regular(path.join(path.dirname(path.dirname(inputPath)), "pre-rc-qualification.json"))),
    "selected source qualification differs from reviewed input");
}

export function boundedWebPhases(phases, spec) {
  const native = spec.values?.native_root, disposition = native && path.join(native, "docs/sprints/release_1_2_0_binding.md");
  if (spec.release.version !== "1.2.0" || spec.values.personal_bin || spec.values.community_bin || !disposition || !existsSync(disposition)) return phases;
  const { locations } = reviewedCompositionLocations(native), inputPath = path.join(locations.W, "qualification-input.json");
  const outputRoot = path.join(spec.records_root, "web-admission");
  if (locations.W === outputRoot) return phases;
  const input = JSON.parse(regular(inputPath)), repository = spec.values.web_test_root;
  const selectedComponents = JSON.parse(regular(spec.values.web_components));
  if (path.dirname(spec.values.web_components) === spec.records_root) {
    const changed = productComponents.some(key => selectedComponents[key] !== input.components[key]);
    if (changed && !isApprovedReplacement(selectedComponents, input)) return phases;
    if (!changed && !qualificationCarrier(JSON.parse(regular(spec.values.accepted_input_path)))) return phases;
  }
  const recovery = recoveryLocations(inputPath, repository, selectedComponents.web_test);
  const applicability = validateCompositionSelection(spec, inputPath);
  assert.equal(input.execution.random_seed, 20261029); assert.equal(input.execution.random_groups, 3000);
  assert.equal(String(input.execution.random_seed), spec.values.web_seed);
  const phase = phases.find(row => row.id === "web_admission");
  const closeouts = phases.find(row => row.id === "component_release")?.actions.filter(action => action.type === "ait_closeout");
  assert.equal(closeouts?.length, 5, "composition requires the five accepted source closeouts");
  for (const row of spec.collections.component_closeouts) {
    const selected = closeouts.filter(action => action.id === `closeout-${row.id}`);
    assert.equal(selected.length, 1);
    for (const key of ["task", "snapshot", "repository_root", "edit_root", "remote", "patchset", "finish_local_before_ready"])
      assert.deepEqual(selected[0][key], row[key], `compiled ${row.id} closeout differs from accepted source`);
  }
  assert(phase); assert.deepEqual(phase.actions.map(action => action.id), ["bind-web-admission-input", "run-web-admission"], "bounded replacement requires the existing Web recipe");
  const [bind, admission] = phase.actions;
  const argument = (action, flag) => {
    assert.equal(action.argv.filter(value => value === flag).length, 1); return action.argv[action.argv.indexOf(flag) + 1];
  };
  const originalInput = argument(admission, "--input"), originalCli = argument(admission, "--ait"), originalServer = argument(admission, "--server-bin");
  const corrected = JSON.parse(regular(spec.values.web_components)).web_test !== input.components.web_test;
  assert.equal(bind.outputs[0], originalInput); assert.equal(argument(admission, "--output-dir"), outputRoot);
  assert.equal(argument(admission, "--repository"), repository);
  const workingInput = corrected ? originalInput : inputPath;
  const selectedCli = applicability ? originalCli : input.executables.ait.path, selectedServer = applicability ? originalServer : input.executables.server.path;
  const replace = argv => argv.map(value => value === originalInput ? workingInput : value === originalCli ? selectedCli : value === originalServer ? selectedServer : value);
  const run = { ...admission, argv: replace(admission.argv), probe: { ...admission.probe, argv: replace(admission.probe.argv) } };
  const program = "const [module,...args]=process.argv.slice(1); const {pathToFileURL}=await import('node:url'); const {runBoundedWebAction}=await import(pathToFileURL(module)); await runBoundedWebAction(...args);";
  const verification = "const [module,...args]=process.argv.slice(1); const {pathToFileURL}=await import('node:url'); const {verifyBoundedWebAction}=await import(pathToFileURL(module)); verifyBoundedWebAction(...args);";
  const selectedOutput = name => path.join(outputRoot, "results", name);
  const supplemental = recoveryEvidence(recovery);
  const actions = names.map((name, index) => {
    const helper = { "native.current.reviewed-fork": "native-fork.sh", "native.current.two-authorities": "run-native-authority-isolation.mjs",
      randomized_3000: "run-randomized-commands.sh" }[name];
    const outputs = [selectedOutput(name)];
    const verifier = { cwd: repository, argv: [process.execPath, "--input-type=module", "-e", verification,
      path.join(native, "ci/release_web_bounded.mjs"), name, workingInput, repository, selectedCli,
      selectedServer, outputs[0], outputRoot, ...supplemental] };
    return { id: `bounded-web-${index + 1}`, type: "command", mutation: false, cwd: repository, outputs,
      ...(recovery && !(applicability && ["ait.semantic.init", "closeout"].includes(name)) ? { adopt: verifier, probe: verifier } : {}),
      argv: [process.execPath, "--input-type=module", "-e", program, path.join(native, "ci/release_web_bounded.mjs"), name,
        workingInput, repository, selectedCli, selectedServer, outputs[0],
        ...(applicability && name === "ait.semantic.init" ? [spec.values.public_source] : []),
        ...(helper ? [path.join(repository, "acceptance", helper)] : []),
        ...(name === "closeout" ? names.slice(0, 4).map(earlier => path.join(selectedOutput(earlier), earlier === "randomized_3000" && recovery ? "commands.tsv" : earlier === "randomized_3000" ? "summary.json" : "cases.tsv")) : []),
        ...supplemental] };
  });
  const currentBind = { ...bind, argv: replace(bind.argv), probe: bind.probe && { ...bind.probe, argv: replace(bind.probe.argv) } };
  const caches = applicability ? qualificationCacheActions(native, spec.records_root, spec.values.github_repository) : [];
  return phases.map(row => row === phase ? { ...row, actions: [...(corrected ? [currentBind] : []), ...actions, run] } :
    applicability && row.id === "clean_host_qualification" ? { ...row, actions: [...row.actions, ...caches] } : row);
}

export function verifyBoundedWebAction(name, inputPath, repository, cli, server, output, outputRoot, ...supplemental) {
  assert(names.includes(name));
  const input = JSON.parse(regular(inputPath));
  for (const [key, file] of [["ait", cli], ["server", server]]) {
    assert.equal(input.executables[key].path, file); assert.equal(hash(regular(file)), input.executables[key].sha256);
  }
  const { context, matrix } = validateComposedWebInputs(inputPath, input, repository, outputRoot);
  const recovery = recoveryLocations(context.priorInputPath, repository, context.assertionCorrection?.snapshot ?? input.components.web_test);
  assert(recovery, "adoption requires the finite reviewed recovery inventory");
  assert.deepEqual(supplemental, recoveryEvidence(recovery));
  for (const directory of [recovery.initial, recovery.dual, recovery.collection, recovery.fork, recovery.final, ...recovery.priorRoots].filter(Boolean)) context.pinnedTree(directory);
  for (const file of [...recovery.ownerFiles, ...recovery.runtimeFiles]) context.pinned(file);
  verifyBoundedCase(context, name, output, matrix, true);
  for (const file of Object.keys(context.plan.source.control_files)) context.pinned(file);
}

// Fixed Stage 5 dependencies, in memory only. Native receipt transactions still
// own every action output. A failure blocks its actual dependents, not peers.
export async function runBoundedWebBatch(root, planSha, release, actions, completed = () => {}) {
  assert.deepEqual(actions.map(action => action.id), names.map((_, index) => `bounded-web-${index + 1}`));
  mkdirSync(path.join(root, "conductor-logs"), { recursive: true });
  for (const action of actions) for (const output of action.outputs) mkdirSync(path.dirname(output), { recursive: true });
  const outcomes = Array(5).fill("blocked");
  const execute = async index => {
    const action = actions[index];
    const context = { plan_sha256: planSha, release, action, replacements: {} };
    const log = path.join(root, "conductor-logs", action.id + ".log");
    const program = "const {pathToFileURL}=require('node:url'); (async()=>{const {runReceiptTransaction}=await import(pathToFileURL(process.argv[1]));const [root,id,context,action,log]=JSON.parse(process.argv[2]);const r=runReceiptTransaction(root,id,context,action,{},log);process.exit(r.status??65);})().catch(e=>{console.error(e.message);process.exit(65);});";
    try {
      const status = await new Promise((resolve, reject) => {
        const child = spawn(process.execPath, ["-e", program, path.join(here, "release_receipt_transaction.mjs"),
          JSON.stringify([root, action.id, context, action, log])], { cwd: action.cwd, stdio: ["ignore", "ignore", "pipe"] });
        let diagnostic = "";
        child.stderr.on("data", bytes => { diagnostic = (diagnostic + bytes).slice(-16384); });
        child.once("error", reject); child.once("close", code => {
          if (code !== 0 && !existsSync(log)) writeFileSync(log, diagnostic);
          resolve(code);
        });
      });
      outcomes[index] = status === 0 ? "pass" : "fail";
    } catch (error) { outcomes[index] = "fail"; if (!existsSync(log)) writeFileSync(log, String(error)); }
    completed(index, outcomes[index]); return outcomes[index];
  };
  const isolation = execute(2);
  await execute(0);
  // Fork owns its actor/config/authority preflight. A failed owner bootstrap
  // assertion does not invalidate an otherwise eligible Developer Task setup.
  await execute(1);
  // The adapter verifies actual clean state before random even if init/fork
  // failed. An unconfirmed cleanup blocks campaign execution at that boundary.
  await execute(3);
  await isolation;
  if (outcomes.slice(0, 4).every(value => value === "pass")) await execute(4); else completed(4, "blocked");
  return outcomes;
}

function captureCleanState(output, inputPath, input, repository, cli, env, fresh = false) {
  for (const [file, args] of [["final-config.json", ["config", "show", "--json"]], ["final-status.json", ["status", "--json", "--full"]],
    ["final-queue.json", ["queue", "summary", "--remote", "origin", "--json"]]]) {
    if (!fresh) writeFileSync(path.join(output, file), call([cli, ...args], { cwd: repository, env }));
    else {
      const result = spawnSync(cli, args, { cwd: repository, env, encoding: "utf8", maxBuffer: 16 * 1024 * 1024 });
      const prefix = path.join(output, file.slice(0, -5));
      writeFileSync(prefix + ".argv", [cli, ...args].join("\n") + "\n"); writeFileSync(prefix + ".stdout", result.stdout ?? "");
      writeFileSync(prefix + ".stderr", result.stderr ?? ""); writeFileSync(prefix + ".exit", String(result.status ?? 65) + "\n");
      assert.equal(result.status, 0, `fresh final-state read failed: ${result.stderr}`); writeFileSync(path.join(output, file), result.stdout);
    }
  }
  writeFileSync(path.join(output, "internal-change-argv.tsv"), "");
  const baseline = JSON.parse(regular(path.join(path.dirname(inputPath), "evidence/starting-config.json")));
  validateFinalWebState(output, input, baseline);
}

function soloFunctions(repository) {
  const source = regular(path.join(repository, "acceptance/run-ait-solo-local.sh")).toString();
  const functions = [...source.matchAll(/^awt_[a-z_]+\(\) \{\n[\s\S]*?^\}/gm)].map(match => match[0]);
  for (const name of ["awt_semantic_case", "awt_ensure_mutation_task", "awt_abandon_mutation_task", "awt_json", "awt_restore_solo_local_config"])
    assert.equal(functions.filter(body => body.startsWith(name + "() {")).length, 1, `frozen solo helper is missing or ambiguous: ${name}`);
  return functions.join("\n");
}

export async function runBoundedWebAction(name, inputPath, repository, cli, server, output, ...dependencies) {
  assert(names.includes(name));
  const input = JSON.parse(regular(inputPath));
  const approved = isApprovedReplacement(input.components, { components: previousCohort });
  const expectedDependencies = name === "closeout" ? 4 : ((approved && name === "ait.semantic.init") ||
    ["native.current.reviewed-fork", "native.current.two-authorities", "randomized_3000"].includes(name)) ? 1 : 0;
  dependencies = dependencies.slice(0, expectedDependencies);
  assert.equal(webAcceptanceTree(path.join(repository, "acceptance")).sha256, input.web_test.acceptance_set_sha256);
  for (const [key, file] of [["ait", cli], ["server", server]]) {
    assert.equal(input.executables[key].path, file); assert.equal(hash(regular(file)), input.executables[key].sha256);
  }
  assert(!existsSync(output), "bounded output already exists; do not repeat runtime work");
  // The receipt output stays inside this operation. Prior noncanonical raws
  // are copied only after their complete pinned inventory and assertions pass.
  // The ordinary post-run probe verifies those same bytes before committing.
  let operation = path.dirname(output);
  while (operation !== path.dirname(operation) && !existsSync(path.join(operation, "conductor-plan.json"))) operation = path.dirname(operation);
  const outputRoot = existsSync(path.join(operation, "conductor-plan.json")) ? path.join(operation, "web-admission") : path.dirname(path.dirname(output));
  const { locations } = reviewedCompositionLocations(path.dirname(here));
  const priorInputPath = path.join(locations.W, "qualification-input.json");
  const recovery = recoveryLocations(priorInputPath, repository, input.components.web_test);
  const source = !(approved && ["ait.semantic.init", "closeout"].includes(name)) && recovery && ({ "ait.semantic.init": recovery.initial, "native.current.two-authorities": recovery.dual,
    randomized_3000: recovery.collection, "native.current.reviewed-fork": recovery.fork, closeout: recovery.final }[name]);
  if (source) {
    const { context, matrix } = validateComposedWebInputs(inputPath, input, repository, outputRoot);
    context.pinnedTree(source); verifyBoundedCase(context, name, source, matrix, true);
    cpSync(source, output, { recursive: true }); return;
  }
  const env = { ...process.env, AIT_WEB_TEST_REPOSITORY_ROOT: repository, AIT_WEB_TEST_AIT_BIN: cli,
    AIT_WEB_TEST_SERVER_BIN: server, AIT_WEB_TEST_PERSONAL_BIN: "", AIT_WEB_TEST_COMMUNITY_BIN: "", AIT_WEB_TEST_COMMUNITY_CLI_BIN: "",
    AIT_WEB_TEST_RESULTS_DIR: output, AIT_WEB_TEST_EXPECTED_MODE: "solo_local" };
  if (approved && name === "ait.semantic.init") {
    const { context, matrix } = validateComposedWebInputs(inputPath, input, repository, outputRoot);
    assert(context.applicability); assert.equal(dependencies[0], context.accepted.paths.public_source); mkdirSync(output, { recursive: true });
    const script = `set -eu
awt_script_dir=$1; export awt_script_dir
. "$awt_script_dir/lib.sh"
awt_require_tools jq cksum
cd "$AIT_WEB_TEST_REPOSITORY_ROOT"
: >"$AIT_WEB_TEST_RESULTS_DIR/cases.tsv"
awt_run_case ait.semantic.init awt_capture_json_success ait.semantic.init 'type == "object"' "$2" --input-type=module -e "$3" "$4" "$5" "$6" "$7" "$8"
`;
    call(["sh", "-s", "--", path.join(repository, "acceptance"), process.execPath, teamCaptureProgram,
      path.join(context.accepted.paths.native_root, "ci/release_core_team_cases.mjs"), dependencies[0], cli,
      input.release.version, context.candidate.release.source_commit], { cwd: repository, env, input: script });
    verifyBoundedCase(context, name, output, matrix, true); return;
  }
  if (name === "randomized_3000") {
    assert.equal(dependencies[0], path.join(repository, "acceptance/run-randomized-commands.sh"));
    const temporary = mkdtempSync(path.join(repository, ".ait-runtime/stage5-random-")), runtime = path.join(temporary, "run");
    const clean = path.join(temporary, "clean"); mkdirSync(clean);
    captureCleanState(clean, inputPath, input, repository, cli, env);
    // The WT runner owns its allowed runtime path. Only this invocation's
    // completed output is copied into the native transaction's staging path.
    call(["sh", dependencies[0], "--repository", repository, "--mode", "solo_remote", "--groups", "3000", "--seed", String(input.execution.random_seed),
      "--ait", cli, "--output-dir", runtime], { cwd: repository, env });
    cpSync(runtime, output, { recursive: true }); rmSync(temporary, { recursive: true }); return;
  }
  mkdirSync(output, { recursive: true });
  if (name === "closeout") {
    assert.equal(dependencies.length, 4);
    for (const file of dependencies) regular(file);
    const scan = file => {
      const st = lstatSync(file); assert(!st.isSymbolicLink());
      if (st.isDirectory()) for (const child of readdirSync(file)) scan(path.join(file, child));
      else if (file.endsWith(".argv")) assert(!/[A-Z]+T-[0-9]{4}\/C-[0-9]{2}/.test(regular(file).toString()), "internal Change argv leaked");
    };
    for (const file of dependencies) scan(path.dirname(file));
    captureCleanState(output, inputPath, input, repository, cli, env, approved);
    if (approved) { const { context, matrix } = validateComposedWebInputs(inputPath, input, repository, outputRoot);
      verifyBoundedCase(context, name, output, matrix, true); }
    return;
  }
  const common = `set -eu
awt_script_dir=$1; export awt_script_dir
. "$awt_script_dir/lib.sh"
awt_require_tools jq curl cksum
cd "$AIT_WEB_TEST_REPOSITORY_ROOT"
: >"$AIT_WEB_TEST_RESULTS_DIR/cases.tsv"
`;
  if (name === "native.current.two-authorities") {
    assert.equal(dependencies[0], path.join(repository, "acceptance/run-native-authority-isolation.mjs"));
    call(["sh", "-s", "--", path.join(repository, "acceptance"), dependencies[0]], { cwd: repository, env,
      input: common + 'awt_run_case native.current.two-authorities node "$2" "$AIT_WEB_TEST_AIT_BIN" "$AIT_WEB_TEST_SERVER_BIN" "$AIT_WEB_TEST_RESULTS_DIR/native-two-authorities"\n' }); return;
  }
  assert(["ait.semantic.init", "native.current.reviewed-fork"].includes(name));
  if (name === "native.current.reviewed-fork") assert.equal(dependencies[0], path.join(repository, "acceptance/native-fork.sh"));
  const setup = common + soloFunctions(repository) + `
. "$awt_script_dir/native-fork.sh"
awt_expected_mode=solo_local; awt_mode_label=solo-local
awt_mutation_task_id=; awt_mutation_change_ref=; awt_mutation_worktree=; awt_mutation_worktree_name=
awt_mutation_line=; awt_mutation_parent=; awt_mutation_head=; awt_started_task_ids=
awt_marker_path=fixtures/.ait-web-test-local-probe.txt
awt_marker_a=$awt_script_dir/fixtures/local-marker-a.txt; awt_marker_b=$awt_script_dir/fixtures/local-marker-b.txt
awt_validate_repository "$AIT_WEB_TEST_REPOSITORY_ROOT" "$AIT_WEB_TEST_AIT_BIN" solo_local
awt_config_capture ait.semantic
awt_bounded_cleanup() {
  awt_cleanup_status=0
  for awt_owned_task in $awt_started_task_ids; do
    awt_mutation_task_id=$awt_owned_task
    awt_abandon_mutation_task || awt_cleanup_status=1
  done
  awt_restore_solo_local_config || awt_cleanup_status=1
  return "$awt_cleanup_status"
}
awt_config_guard awt_bounded_cleanup
`;
  const init = 'awt_run_case ait.semantic.init awt_semantic_case init\n';
  const fork = `
awt_before=$(awt_state_digest)
awt_ensure_mutation_task
awt_case=ait.semantic.change.create
cp "$awt_marker_a" "$awt_mutation_worktree/$awt_marker_path"
awt_json "$awt_case.review-anchor" '.snapshot_id != null and .message == "ait-web-test-reviewed-fork-anchor"' \
  awt_in_dir "$awt_mutation_worktree" "$AIT_WEB_TEST_AIT_BIN" snapshot create "$awt_mutation_task_id" --message ait-web-test-reviewed-fork-anchor --json --full
awt_reviewed_snapshot=$(jq -er '.snapshot_id' "$(awt_case_stdout_path "$awt_case.review-anchor")")
awt_native_reviewed_fork "$awt_case" "$awt_mutation_task_id" "$awt_mutation_worktree_name" "$awt_mutation_worktree" "$awt_reviewed_snapshot"
cp "$awt_marker_b" "$awt_mutation_worktree/$awt_marker_path"
awt_json "$awt_case.correction" ".task_id == \\"$awt_mutation_task_id\\" and .change_id == \\"$awt_fork_change_id\\" and .snapshot_id != \\"$awt_reviewed_snapshot\\"" \
  awt_in_dir "$awt_mutation_worktree" "$AIT_WEB_TEST_AIT_BIN" snapshot create "$awt_mutation_task_id" --message ait-web-test-reviewed-fork-correction --json --full
awt_native_fork_preserved
awt_bounded_cleanup
awt_started_task_ids=
awt_after=$(awt_state_digest)
awt_append_case_result native.current.reviewed-fork pass 0 "$awt_before" "$awt_after"
`;
  call(["sh", "-s", "--", path.join(repository, "acceptance")], { cwd: repository, env,
    input: setup + (name === "ait.semantic.init" ? init : fork) + '\nawt_bounded_cleanup\ntrap - EXIT HUP INT TERM\n' });
}

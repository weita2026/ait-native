// Consume the reviewed 1.2.0 assertion disposition. Nothing here executes a
// qualification or creates evidence, a completion receipt or a result schema.
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { existsSync, lstatSync, readFileSync, readdirSync, realpathSync, mkdtempSync, rmSync } from "node:fs";
import path from "node:path";
import { tmpdir } from "node:os";
import { fileURLToPath } from "node:url";
import { spawnSync } from "node:child_process";
import { digest, recoverReceiptTransaction } from "./release_receipt_transaction.mjs";
import { packageIdentity, workflowResult, packageAcceptance } from "./release_native_workflow.mjs";
import { installedCoreCases, requireExecutedCoreCase } from "./release_core_installed_cases.mjs";
import { recoveryLocations, recoveryEvidence } from "./release_web_recovery_inputs.mjs";
import { validateRecoveredRandom } from "./release_web_random_coverage.mjs";
import { priorWebInput, correctedWTIdentity, sameProductComponents } from "./release_web_recovery_identity.mjs";
import { replacementIdentity, currentProducedInputs, verifyQualificationCache, cacheDirectory, affectedCommand } from "./release_web_applicability.mjs";
import { verifyInstalledTeamCaptures } from "./release_core_team_cases.mjs";

const hash = bytes => createHash("sha256").update(bytes).digest("hex");
function authorityDigest(value) {
  const match = typeof value === "string" && /^(0|[1-9][0-9]{0,9}):(0|[1-9][0-9]{0,19})$/.exec(value);
  assert(match && BigInt(match[1]) <= 0xffffffffn && BigInt(match[2]) <= 0xffffffffffffffffn, "authority digest is malformed");
}
export function regular(file) {
  const st = lstatSync(file);
  assert(st.isFile() && !st.isSymbolicLink(), `evidence must be regular: ${file}`);
  return readFileSync(file);
}
const json = file => JSON.parse(regular(file));
const below = (file, root) => file.startsWith(root + path.sep);
const allActions = plan => plan.phases.flatMap(p => p.actions);
const phases = ["source_preflight", "component_release", "candidate_freeze", "public_qualification", "candidate_admission",
  "component_receipts", "clean_host_qualification", "web_admission", "tag", "protected_promotion", "endpoint_publication",
  "winget_submission", "latest_alias", "closeout"];
// Exact reviewed command families; a future/new leaf cannot inherit this
// disposition merely because an aggregate says core_quality=pass.
const reviewed = {
  line: "list|create|switch|show|archive|rename|delete|merge|cleanup",
  git: "import|export|mirror|migrate|recover|restore", doctor: "memory-root|runtime-root|plan-authority", queue: "summary",
  remote: "add|list|recover-head", release: "candidate create|check|build|native-source|native-bundle|package|formula|show|publish|promote",
  repo: "show|retire|restore|jobs|ci-capabilities", config: "show|set|unset", external: "update|status|doctor|link|unlink",
  gc: "stats|validate|prune", stash: "save|list|show|apply|pop|drop", plan: "list|show|revisions|items|candidates|inspect|sync",
  task: "quick|start|list|show|audit|finish|abandon", change: "create|list|show|revert|replay|close|publish",
  snapshot: "create|list|show|diff|restore-lines|revert|replay|ancestry|is-ancestor|merge-base", tag: "create|list|show|delete",
  patchset: "publish|list|show|select|ci-status|rerun-ci",
  review: "show|team request|team approve|team request-changes|team comment|team defer|task approve|task request-changes|task comment|task defer|code submit|code template",
  attest: "put|show", policy: "eval|show|waive", worktree: "status|restore|show|path|doctor|cleanup-candidates|cleanup|prune-stale|list|sync|recreate|recover-task|restore-owned-head|rebase|remove",
  team: "contacts|session ensure|session acknowledge", workflow: "guide|reconcile|ready|finish",
};
const reviewedLeaves = new Set(["blame", "status", "diff", "pull", "push", "commit", "help",
  ...Object.entries(reviewed).flatMap(([family, children]) => children.split("|").map(child => `${family} ${child}`))]);
export const sourceAssertions = Object.freeze([
  "main_app::tests::agent_action_full_projection_requires_json_on_each_supported_command",
  "compact_agent_action_json_drives_the_complete_local_task_loop",
  "native_git_friendly_aliases_preserve_snapshot_and_line_contracts",
  "native_line_delete_tombstones_ref_preserves_snapshot_and_changes_identity_on_reuse",
  "native_line_rename_preserves_identity_and_updates_default_current_pointers",
  "native_line_merge_distinguishes_fast_forward_equal_and_already_contains",
  "native_line_cleanup_previews_by_default_and_requires_yes_to_archive",
  "native_ait_snapshots_export_deterministically_and_roundtrip_back_through_ait",
  "native_git_import_bootstrap_dry_run_is_inert_and_invalid_setup_blocks_before_history",
  "native_git_mirror_independent_inbound_outbound_changes_and_raw_tags_converge",
  "native_blame_is_read_only_and_snapshot_restore_lines_is_preview_first",
  "doctor_plan_authority_runs_without_repo", "doctor_runtime_root_reports_inside_repo_as_snapshot_protected_warning",
  "native_remote_list_reports_configured_remotes", "native_repo_show_uses_fixed_authority",
  "family_release_cli_rejects_wrong_channel_and_missing_receipts",
  "config_surface::tests::admitted_overrides_set_and_unset_without_erasing_inert_legacy_data",
  "external_surface::tests::external_link_rejects_a_name_missing_from_the_root_manifest",
  "external_surface::tests::external_update_validate_reports_binding_validation_findings",
  "gc_binary_runtime_exposes_only_supported_maintenance_operations",
  "native_stash_cli_preserves_the_complete_binary_db_lifecycle", "local_plan_commands_work_end_to_end",
  "native_task_audit_cli_local_default_does_not_fallback_to_remote",
  "native_change_namespace_supports_local_and_remote_scopes", "native_change_namespace_supports_revert_and_replay",
  "native_snapshot_namespace_supports_list_show_and_diff", "native_snapshot_revert_and_replay_work_without_snapshot_files_view",
  "native_tag_namespace_stores_message_and_resolves_snapshot_refs",
  "native_patchset_namespace_supports_list_show_and_select",
  "native_review_namespace_supports_distinct_code_task_team_and_template_lanes",
  "native_policy_namespace_supports_show_and_waive",
  "native_worktree_removal_surfaces_share_yes_and_dry_run_contract",
  "native_worktree_capture_authority_show_rejects_authority_mismatches_and_accepts_zero",
  "native_workflow_reconcile_inventory_is_read_only_and_stable",
]);
const arg = (action, flag) => {
  assert.equal(action.argv.filter(v => v === flag).length, 1, `ambiguous ${flag}`);
  return action.argv[action.argv.indexOf(flag) + 1];
};
function planAt(directory) {
  for (let root = directory; root !== path.dirname(root); root = path.dirname(root)) {
    const file = path.join(root, "conductor-plan.json");
    if (existsSync(file)) { const bytes = regular(file); return { root, file, bytes, plan: JSON.parse(bytes) }; }
  }
  throw new Error("composition requires its reviewed replacement Plan");
}

export function compositionContext(inputPath, outputRoot, repository, preparedPlan) {
  const inside = path.dirname(inputPath) === outputRoot || below(inputPath, outputRoot);
  if (inside && !preparedPlan && !existsSync(path.join(path.dirname(outputRoot), "conductor-plan.json"))) return null;
  const selected = preparedPlan ?? planAt(outputRoot), { root, plan } = selected;
  if (inside && !allActions(plan).some(action => action.id === "bounded-web-1")) return null;
  assert.equal(plan.contract, "ait.release.conductor-plan/v1");
  assert.deepEqual(plan.phases.map(p => p.id), phases, "replacement Plan phase inventory differs");
  assert(plan.phases.every(p => p.actions.length > 0));
  assert.equal(new Set(allActions(plan).map(a => a.id)).size, allActions(plan).length, "replacement Plan action IDs conflict");
  assert.equal(realpathSync(plan.records_root), realpathSync(root));
  assert.equal(outputRoot, path.join(root, "web-admission"));
  const pins = plan.source?.control_files;
  assert(pins && Object.keys(pins).length > 0);
  const pinned = file => {
    const bytes = regular(file);
    assert.equal(hash(bytes), pins[file], `unbound or changed composition evidence: ${file}`);
    return bytes;
  };
  for (const file of Object.keys(pins)) { assert(path.isAbsolute(file)); pinned(file); }
  pinned(fileURLToPath(import.meta.url));
  let priorInputPath = inputPath, assertionCorrection = null, applicability = null, accepted = null;
  const acceptedPath = path.join(root, "accepted-input.json");
  if (pins[acceptedPath]) {
    accepted = JSON.parse(pinned(acceptedPath));
    const historical = priorWebInput(accepted.paths.native_root, accepted.release);
    if (historical && accepted.web.acceptance_snapshot !== historical.input.components.web_test) {
      const selected = JSON.parse(pinned(accepted.paths.web_components));
      applicability = replacementIdentity(accepted, historical.input, selected);
      assertionCorrection = correctedWTIdentity(accepted, historical.input,
        applicability ? { ...historical.input.components, web_test: selected.web_test } : selected);
      priorInputPath = historical.inputPath;
    }
  }
  assert(!inside || assertionCorrection, "an internal bounded input requires its reviewed WT correction identity");
  const priorInput = JSON.parse(pinned(priorInputPath));
  const input = inside ? JSON.parse(regular(inputPath)) : priorInput;
  const admission = allActions(plan).find(action => action.id === "run-web-admission");
  const workingInputPath = assertionCorrection ? arg(admission, "--input") : priorInputPath;
  if (inside) {
    assert.equal(inputPath, workingInputPath);
    if (!applicability) sameProductComponents(input.components, priorInput);
    else assert.deepEqual(input.components, applicability.components);
    assert.equal(input.components.web_test, assertionCorrection.snapshot);
    assert.equal(input.web_test.head_snapshot_id, assertionCorrection.snapshot);
    assert.equal(input.web_test.acceptance_set_sha256, assertionCorrection.acceptanceSha256);
    for (const key of applicability ? ["contract", "browser_evidence"] : ["contract", "executables", "qualification", "candidate_authority_sha256", "browser_evidence"])
      assert.deepEqual(input[key], priorInput[key], `replacement input changed frozen ${key}`);
    for (const key of applicability ? ["version", "channel"] : ["id", "version", "channel"]) assert.equal(input.release[key], priorInput.release[key]);
    if (applicability) {
      assert.notEqual(input.executables.ait.sha256, priorInput.executables.ait.sha256, "old CLI bytes cannot prove corrected init/config");
      assert.equal(input.server.url, accepted.web.server_url);
    }
    for (const key of ["random_seed", "random_groups"]) assert.equal(input.execution[key], priorInput.execution[key]);
    for (const key of ["order", "result_labels"]) if (priorInput.execution[key]) assert.deepEqual(input.execution[key], priorInput.execution[key]);
  }
  assert.equal(input.contract, "ait.release.web-admission-input/v1");
  assert.equal(input.release.version, "1.2.0"); assert.equal(input.release.channel, "stable");
  assert.deepEqual(Object.keys(input.executables).sort(), ["ait", "server"]);
  assert.deepEqual(input.browser_evidence, {});
  assert.equal(input.web_test.repository_index, 23); assert.equal(input.web_test.namespace, "WT");
  for (const exe of Object.values(input.executables)) assert.equal(hash(regular(exe.path)), exe.sha256, "composition candidate bytes changed");
  const originalRoot = path.dirname(path.dirname(priorInputPath));
  assert.notEqual(root, originalRoot);
  const originalPlan = JSON.parse(pinned(path.join(originalRoot, "conductor-plan.json")));
  assert.equal(originalPlan.contract, "ait.release.conductor-plan/v1"); assert.equal(originalPlan.records_root, originalRoot);
  assert.deepEqual(originalPlan.phases.map(p => p.id), phases);
  const originalState = JSON.parse(pinned(path.join(originalRoot, "conductor-state.json")));
  assert.equal(originalState.contract, "ait.release.conductor-state/v1");
  assert.deepEqual(originalState.phases.map(p => [p.id, p.actions.map(a => a.id)]), originalPlan.phases.map(p => [p.id, p.actions.map(a => a.id)]));
  assert.equal(originalState.plan_sha256, hash(pinned(path.join(originalRoot, "conductor-plan.json"))));
  assert.deepEqual(originalState.release, plan.release); assert.deepEqual(plan.release, originalPlan.release);
  const bind = allActions(originalPlan).find(a => a.id === "bind-web-admission-input");
  assert(bind && bind.outputs.includes(priorInputPath));
  assert.equal(realpathSync(bind.cwd), realpathSync(repository), "composition workspace differs");
  for (const id of ["qualify-public-source", "qualify-frozen-candidate", "bind-web-admission-input"]) {
    const action = allActions(originalPlan).find(a => a.id === id);
    const recorded = originalState.phases.flatMap(p => p.actions).find(a => a.id === id);
    assert(action && recorded?.status === "complete", `earlier producer is incomplete: ${id}`);
    const checkpoint = JSON.parse(pinned(path.join(originalRoot, "conductor-checkpoints", `${id}.json`)));
    assert.equal(checkpoint.plan_sha256, originalState.plan_sha256); assert.deepEqual(checkpoint.action, recorded);
    const proof = JSON.parse(pinned(path.join(originalRoot, "conductor-receipts", `${id}.json`)));
    assert.deepEqual(proof.outputs.map(r => r.path).sort(), Object.keys(recorded.outputs).sort(), "earlier receipt output inventory differs");
    for (const row of proof.outputs) assert.equal(digest(row.path), row.sha256, "earlier producer output changed");
    for (const [file, expected] of Object.entries(recorded.outputs)) assert.equal(digest(file), expected);
    assert(recoverReceiptTransaction(originalRoot, id, { plan_sha256: originalState.plan_sha256, release: plan.release, action, replacements: {} }));
  }
  const candidatePath = arg(bind, "--candidate"), originalCandidate = JSON.parse(pinned(candidatePath));
  assert.equal(originalCandidate.contract, "ait.release.operator.pre-tag-candidate-binding/v1"); assert.equal(originalCandidate.status, "ready_for_immutable_tag");
  assert.deepEqual(originalCandidate.qualification, priorInput.qualification);
  assert.equal(originalCandidate.candidate_authority.sha256, priorInput.candidate_authority_sha256);
  for (const key of ["id", "version", "channel"]) assert.equal(originalCandidate.release[key], priorInput.release[key]);
  const originalQualification = JSON.parse(pinned(path.join(originalRoot, "pre-rc-qualification.json")));
  assert.equal(originalQualification.contract, "ait.release.operator.qualification-binding/v1"); assert.equal(originalQualification.status, "qualified_for_version_only_release");
  const currentBind = allActions(plan).find(action => action.id === "bind-web-admission-input");
  const candidate = inside && applicability ? JSON.parse(regular(arg(currentBind, "--candidate"))) : originalCandidate;
  assert.equal(candidate.contract, "ait.release.operator.pre-tag-candidate-binding/v1");
  assert.equal(candidate.status, "ready_for_immutable_tag");
  if (inside) assert.deepEqual(input.release, JSON.parse(JSON.stringify({ ...candidate.release,
    repository: candidate.release.repository ?? candidate.candidate_authority?.record?.publisher?.repository })));
  assert.deepEqual(candidate.qualification, input.qualification);
  assert.equal(candidate.candidate_authority.sha256, input.candidate_authority_sha256);
  for (const key of ["id", "version", "channel"]) assert.equal(candidate.release[key], input.release[key]);
  const qualification = inside && applicability ? JSON.parse(regular(path.join(root, "pre-rc-qualification.json"))) : originalQualification;
  assert.equal(qualification.contract, "ait.release.operator.qualification-binding/v1");
  assert.equal(qualification.status, "qualified_for_version_only_release");
  for (const step of ["solo_local", "solo_remote", "randomized_3000", "closeout"]) assert(!existsSync(path.join(originalRoot, "web-admission/receipts", `${step}.json`)),
    "original stopped admission has conflicting later results");
  const onePin = suffix => {
    const files = Object.keys(pins).filter(f => f.endsWith(suffix));
    assert.equal(files.length, 1, `missing or ambiguous reviewed evidence: ${suffix}`);
    return files[0];
  };
  const disposition = pinned(onePin("/docs/sprints/release_1_2_0_binding.md")).toString();
  const section = disposition.indexOf("Stage 5 assertion disposition — 2026-11-01");
  assert(section >= 0, "reviewed assertion disposition is not bound");
  assert(disposition.slice(section).includes(hash(pinned(priorInputPath))), "input is outside the reviewed assertion disposition");
  const pinnedTree = directory => {
    const files = [];
    const walk = file => {
      const st = lstatSync(file); assert(!st.isSymbolicLink(), "evidence tree has a symlink");
      if (st.isDirectory()) for (const name of readdirSync(file).sort()) walk(path.join(file, name));
      else { pinned(file); files.push(file); }
    };
    walk(directory);
    assert.deepEqual(files.sort(), Object.keys(pins).filter(f => below(f, directory)).sort(), "reviewed evidence file set changed");
  };
  const result = name => {
    const recovery = recoveryLocations(priorInputPath, repository, assertionCorrection?.snapshot ?? input.components.web_test);
    const directory = path.join(outputRoot, "results", name);
    const matches = allActions(plan).filter(a => a.outputs?.includes(directory));
    assert.equal(matches.length, 1, `missing or ambiguous bounded action: ${name}`);
    const action = matches[0];
    assert.equal(action.type, "command"); assert.equal(action.mutation, false);
    assert(action.outputs.every(file => path.isAbsolute(file) && (file === directory || below(file, path.join(outputRoot, "results")))), "bounded output escaped its reviewed result area");
    const fresh = applicability && ["ait.semantic.init", "closeout"].includes(name);
    if (fresh) assert(!action.adopt && !action.probe, "fresh affected execution must not adopt orphan captures");
    if (recovery) for (const value of recoveryEvidence(recovery)) {
      assert(action.argv.includes(value) && (fresh || action.adopt?.argv.includes(value) && action.probe?.argv.includes(value)), "bounded action omits a reviewed supplemental input");
    }
    assert.equal(realpathSync(action.cwd), realpathSync(repository));
    for (const value of [workingInputPath, repository, input.executables.ait.path]) assert(action.argv.includes(value), "bounded action omits immutable invocation input");
    if (name === "native.current.two-authorities") assert(action.argv.includes(input.executables.server.path), "isolation action omits the selected Server input");
    const helper = { "native.current.reviewed-fork": "native-fork.sh", "native.current.two-authorities": "run-native-authority-isolation.mjs" }[name];
    if (helper) assert(action.argv.includes(path.join(repository, "acceptance", helper)), "bounded action omits its pinned WT helper");
    if (name === "closeout") for (const earlier of ["ait.semantic.init", "native.current.reviewed-fork", "native.current.two-authorities", "randomized_3000"]) {
      const directory = path.join(outputRoot, "results", earlier);
      const before = allActions(plan).filter(a => a.outputs?.includes(directory));
      assert.equal(before.length, 1); assert(allActions(plan).indexOf(before[0]) < allActions(plan).indexOf(action), "final state precedes a required gap");
      assert(action.argv.includes(path.join(directory, earlier === "randomized_3000" && recovery ? "commands.tsv" : earlier === "randomized_3000" ? "summary.json" : "cases.tsv")), "final state is not bound to the completed gap outputs");
    }
    const receipt = path.join(root, "conductor-receipts", `${action.id}.json`);
    assert(existsSync(receipt), `bounded action is pending: ${name}`);
    const proof = json(receipt);
    assert.equal(proof.plan_sha256, hash(selected.bytes)); assert.deepEqual(proof.release, plan.release);
    assert.deepEqual(proof.outputs.map(row => row.path).sort(), [...action.outputs].sort(), "bounded receipt output inventory differs");
    // Require published outputs before recovery; this verifier never recovers
    // missing files by renaming a transaction's staged output.
    for (const row of proof.outputs) assert.equal(digest(row.path), row.sha256, "bounded output changed");
    assert(recoverReceiptTransaction(root, action.id, { plan_sha256: hash(selected.bytes), release: plan.release, action, replacements: {} }));
    return directory;
  };
  const context = { ...selected, inputPath, input, priorInputPath, priorInput, workingInputPath, assertionCorrection, applicability, accepted,
    outputRoot, repository, originalRoot, originalPlan, originalState, originalCandidate, originalQualification,
    candidate, qualification, pinned, pinnedTree, onePin, result };
  if (inside && applicability) {
    context.current = currentProducedInputs(context);
    assert(regular(path.join(root, "candidate.json")).equals(context.current.pinned(path.join(root, "candidate.json"))));
    assert(regular(path.join(root, "pre-rc-qualification.json")).equals(context.current.pinned(path.join(root, "pre-rc-qualification.json"))));
    for (const [kind, document] of [["source", qualification], ["candidate", candidate]])
      verifyQualificationCache(cacheDirectory(root, kind), document, kind, accepted.github_repository);
    verifyQualificationCache(path.join(root, "candidate-cache"), candidate, "distribution", accepted.github_repository);
  }
  return context;
}

export function retainedContext(c) {
  if (!c.applicability) return c;
  const selected = recoveryLocations(c.priorInputPath, c.repository, c.assertionCorrection.snapshot);
  const file = selected?.runtimeFiles.find(file => file.endsWith("/qualification-input.json"));
  assert(file, "approved replacement requires the original normally produced retained-runtime input");
  const input = JSON.parse(c.pinned(file)); sameProductComponents(input.components, c.priorInput);
  return { ...c, root: c.originalRoot, input, candidate: c.originalCandidate, qualification: c.originalQualification };
}

function archive(context, expectedDigest, current = false) {
  assert.match(expectedDigest, /^sha256:[0-9a-f]{64}$/);
  const files = Object.entries(current ? context.current.pins : context.plan.source.control_files).filter(([f, h]) => f.endsWith(".zip") && h === expectedDigest.slice(7)).map(([f]) => f);
  assert.equal(files.length, 1, "provider artifact is absent or ambiguous");
  (current ? context.current.pinned : context.pinned)(files[0]);
  // Read only. Verify names and retain exact decompressed bytes; never unpack
  // an artifact into the operation or accept an unrelated local JSON copy.
  const result = spawnSync("python3", ["-c", `import zipfile,json,sys,base64,stat
with zipfile.ZipFile(sys.argv[1]) as z:
 names=[i.filename for i in z.infolist()]
 assert len(names)==len(set(names))
 assert all(not n.startswith('/') and '..' not in n.split('/') and '\\\\' not in n for n in names)
 assert all(not stat.S_ISLNK(i.external_attr >> 16) for i in z.infolist())
 print(json.dumps({i.filename:base64.b64encode(z.read(i)).decode() for i in z.infolist() if not i.is_dir() and (i.filename.endswith('.json') or i.filename=='Canonical core quality gate/4_Run the canonical core quality module.txt')}))`, files[0]],
  { encoding: "utf8", maxBuffer: 64 * 1024 * 1024 });
  assert.equal(result.status, 0, `provider artifact unreadable: ${result.stderr}`);
  const members = JSON.parse(result.stdout);
  return name => { assert(Object.hasOwn(members, name), `provider artifact missing ${name}`); return Buffer.from(members[name], "base64"); };
}

function verifyFreshQualification(c) {
  assert(c.applicability && c.current);
  assert.notEqual(c.candidate.release.id, c.originalCandidate.release.id, "corrected products require their normally derived candidate identity");
  assert.notEqual(c.candidate.qualification.candidate_artifact_digest, c.originalCandidate.qualification.candidate_artifact_digest);
  assert.notEqual(c.qualification.source.git_commit, c.originalQualification.source.git_commit);
  const qBytes = archive(c, c.qualification.workflow.artifact_digest, true)("ait-release.pre-rc-qualification.json");
  assert.equal(hash(qBytes), c.qualification.workflow.evidence_sha256); const q = JSON.parse(qBytes);
  assert.equal(q.contract, "ait.release.pre-rc-qualification/v1"); assert.equal(q.status, "qualified");
  assert.equal(q.source.git_commit, c.qualification.source.git_commit); assert.equal(q.source.workflow_control_commit, c.qualification.workflow.control_commit);
  for (const key of ["core_quality", "command_inventory", "environment_inventory", "release_controls"]) assert.equal(q.gates[key], "pass");
  for (const key of ["immutable_release_tag_created", "public_endpoint_writes", "release_receipts_created"]) assert.equal(q[key], false);
  const aggregate = archive(c, c.input.qualification.aggregate_artifact_digest, true);
  const bytes = aggregate("ait-release.clean-host-status.json"), e = JSON.parse(bytes);
  assert.equal(hash(bytes), c.input.qualification.aggregate_status_sha256);
  assert.equal(e.contract, "ait.release.clean-host.aggregate/v1"); assert.equal(e.status, "qualified"); assert.deepEqual(e.failures, []);
  assert.equal(c.candidate.qualification.clean_host_rows, 24); assert.equal(e.evidence_inventory.length, 24);
  assert.equal(new Set(e.evidence_inventory.map(row => row.path)).size, 24);
  assert.equal(e.release.candidate_artifact_digest, c.input.qualification.candidate_artifact_digest);
  for (const key of ["id", "source_commit", "version", "channel"]) assert.equal(e.release[key], c.candidate.release[key]);
  for (const entry of e.evidence_inventory) {
    const bytes = aggregate(entry.path), row = JSON.parse(bytes); assert.equal(hash(bytes), entry.sha256);
    assert.equal(row.status, "pass"); assert.deepEqual(row.failures, []); assert.deepEqual(row.release, e.release);
  }
  for (const target of ["aarch64-unknown-linux-gnu", "x86_64-unknown-linux-gnu", "aarch64-pc-windows-msvc", "x86_64-pc-windows-msvc"]) {
    const row = JSON.parse(aggregate(`rows/github-product-${target}.json`)); assert.equal(row.row.target, target);
    const install = row.phases.install, cases = install.observations.installed_core_cases;
    assert.equal(install.status, "pass"); assert.equal(cases.core_snapshot, c.input.components.core);
    assert.equal(cases.source_commit, c.candidate.release.source_commit);
    assert.deepEqual(cases.cases, installedCoreCases.map(([target, selector, mode]) => ({ target, selector, mode: mode ?? "fixture" })));
    for (const [suite, selector] of installedCoreCases) {
      const commands = install.commands.filter(command => command.label === `installed Core ${suite}/${selector}`);
      assert.equal(commands.length, 1); assert.equal(commands[0].exit_status, 0); requireExecutedCoreCase(commands[0].stdout_tail, selector);
    }
  }
  // Reuse the normal full package/binding/install verifier for all 24 rows.
  // Old Q/E archives above remain attributed to their original source/bytes.
  const checker = path.join(c.accepted.paths.native_root, "ci/release_prepublish_verify.mjs"); c.pinned(checker);
  const verified = spawnSync(process.execPath, [checker, "qualify", "--root", path.join(cacheDirectory(c.root, "candidate"), "payload"),
    "--candidate-root", path.join(c.root, "candidate-cache/payload"), "--candidate-artifact-digest", c.input.qualification.candidate_artifact_digest,
    "--aggregate-sha256", c.input.qualification.aggregate_status_sha256], { encoding: "utf8", maxBuffer: 16 * 1024 * 1024 });
  assert.equal(verified.status, 0, `fresh candidate package/binding/install proof failed: ${verified.stderr}`);
  c.current.verify();
}

function priorCoverage(c, matrix, coverage) {
  const claim = (id, source) => { assert(!coverage.has(id), `conflicting coverage: ${id}`); coverage.set(id, source); };
  const carrier = archive(c, c.qualification.workflow.artifact_digest);
  const qBytes = carrier("ait-release.pre-rc-qualification.json"), q = JSON.parse(qBytes);
  assert.equal(hash(qBytes), c.qualification.workflow.evidence_sha256);
  assert.equal(q.contract, "ait.release.pre-rc-qualification/v1"); assert.equal(q.status, "qualified");
  assert.equal(q.source.git_commit, c.qualification.source.git_commit);
  assert.equal(q.source.workflow_control_commit, c.qualification.workflow.control_commit);
  for (const key of ["immutable_release_tag_created", "public_endpoint_writes", "release_receipts_created"]) assert.equal(q[key], false);
  for (const key of ["core_quality", "command_inventory", "environment_inventory", "release_controls"]) assert.equal(q.gates[key], "pass");
  const run = JSON.parse(c.pinned(c.onePin("/q-run.json")));
  assert.equal(run.id, c.qualification.workflow.run_id); assert.equal(run.run_attempt, c.qualification.workflow.run_attempt);
  assert.equal(run.head_sha, q.source.git_commit); assert.equal(run.status, "completed"); assert.equal(run.conclusion, "success");
  const logPath = c.onePin("/q-logs.zip"), log = archive(c, "sha256:" + c.plan.source.control_files[logPath])
    ("Canonical core quality gate/4_Run the canonical core quality module.txt").toString();
  for (const selector of sourceAssertions) {
    const passed = log.split(/\r?\n/).filter(line => line.endsWith(`test ${selector} ... ok`));
    assert.equal(passed.length, 1, `reviewed source assertion did not execute exactly once: ${selector}`);
  }
  const sourceMap = JSON.parse(c.pinned(path.join(c.originalRoot, "qualified-public-source/ait-monorepo-source.json")));
  assert.equal(sourceMap.schema, "ait.release.monorepo-source/v1");
  const core = sourceMap.subtrees.filter(row => row.source_repository === "ait-core");
  assert.equal(core.length, 1); assert.equal(core[0].source_snapshot, c.input.components.core, "source-Q Core differs from installed candidate");
  const aggregate = archive(c, c.input.qualification.aggregate_artifact_digest);
  const eBytes = aggregate("ait-release.clean-host-status.json"), e = JSON.parse(eBytes);
  assert.equal(hash(eBytes), c.input.qualification.aggregate_status_sha256);
  assert.equal(e.contract, "ait.release.clean-host.aggregate/v1"); assert.equal(e.status, "qualified"); assert.deepEqual(e.failures, []);
  assert.equal(e.release.candidate_artifact_digest, c.input.qualification.candidate_artifact_digest);
  for (const key of ["id", "source_commit", "version", "channel"]) assert.equal(e.release[key], c.candidate.release[key]);
  for (const target of ["aarch64-unknown-linux-gnu", "x86_64-unknown-linux-gnu", "aarch64-pc-windows-msvc", "x86_64-pc-windows-msvc"]) {
    const file = `rows/github-product-${target}.json`, bytes = aggregate(file), row = JSON.parse(bytes);
    assert.equal(e.evidence_inventory.find(r => r.path === file)?.sha256, hash(bytes));
    assert.equal(row.status, "pass"); assert.deepEqual(row.failures, []);
    assert.deepEqual(row.release, e.release); assert.equal(row.row.target, target);
    const install = row.phases.install, cases = install.observations.installed_core_cases;
    assert.equal(install.status, "pass"); assert.equal(cases.core_snapshot, c.input.components.core);
    assert.deepEqual(cases.cases, installedCoreCases.map(([suite, selector, mode]) => ({ target: suite, selector, mode: mode ?? "fixture" })));
    assert.equal(cases.source_commit, c.candidate.release.source_commit);
    for (const [suite, selector] of installedCoreCases) {
      const commands = install.commands.filter(r => r.label === `installed Core ${suite}/${selector}`);
      assert.equal(commands.length, 1); assert.equal(commands[0].exit_status, 0);
      requireExecutedCoreCase(commands[0].stdout_tail, selector);
    }
  }
  for (const name of ["local-finish", "explicit-rebase"]) {
    const planPath = c.onePin(`/native-scenes/${name}/conductor-plan.json`), sceneRoot = path.dirname(planPath);
    const scene = JSON.parse(c.pinned(planPath)), action = allActions(scene).find(a => a.type === "ait_closeout");
    assert(action); assert.deepEqual(scene.release, c.plan.release);
    assert.equal(hash(c.pinned(action.ait_bin)), c.input.executables.ait.sha256, "Native scene used other CLI bytes");
    const releaseRoot = path.dirname(path.dirname(sceneRoot));
    assert.equal(hash(c.pinned(path.join(releaseRoot, "candidate-bin/ait-server"))), c.input.executables.server.sha256);
    assert.equal(hash(c.pinned(path.join(releaseRoot, "native-runtime-bin/ait-runner"))),
      hash(c.pinned(path.join(c.originalRoot, "native-runtime-bin/ait-runner"))), "Native scene used other Runner bytes");
    const pinPath = path.join(sceneRoot, "conductor-receipts/component.package-identity.json");
    const pin = packageIdentity(JSON.parse(c.pinned(pinPath)));
    assert.equal(pin.revision_snapshot_id, action.snapshot);
    const finish = JSON.parse(c.pinned(path.join(sceneRoot, "conductor-logs", name === "explicit-rebase" ? "component.ait.finish-recovery.log" : "component.ait.finish.log")));
    workflowResult(finish, "finish", pin, { invocation: { task: action.task, remote: action.remote ?? "origin", pinPath, expectedIdentity: pin } });
    assert.equal(packageAcceptance(finish, pin, action.task)?.complete, true);
    if (name === "explicit-rebase") {
      assert.equal(c.pinned(path.join(sceneRoot, "conductor-logs/component.ait.finish.log")).length, 0);
      const interrupted = JSON.parse(c.pinned(path.join(sceneRoot, "conductor-logs/component.ait.finish-interruption.json")));
      assert.equal(interrupted.status, "interrupted_after_formal_job"); assert.equal(interrupted.stdout_bytes, 0);
      assert.equal(interrupted.worker_job_index, pin.worker_job_index);
    }
  }
  const marker = path.join(c.originalRoot, "web-admission/private-logs/solo_local.marker");
  const directory = c.pinned(marker).toString().trim(); c.pinnedTree(directory);
  assert.equal(realpathSync(directory), directory);
  assert(below(directory, path.join(realpathSync(c.repository), ".ait-runtime/direct-test-results")));
  const rows = c.pinned(path.join(directory, "cases.tsv")).toString().trim().split(/\r?\n/).map(l => l.split("\t"));
  const prefix = ["inventory.ait"];
  for (const [command, kind] of matrix) { prefix.push(`ait.help.${command.replaceAll(" ", ".")}`); if (kind === "leaf") prefix.push(`ait.invalid.${command.replaceAll(" ", ".")}`); }
  assert.deepEqual(rows.slice(0, -1).map(r => r[0]), prefix);
  assert(rows.slice(0, -1).every(r => r.length === 5 && r[1] === "pass" && r[2] === "0"));
  for (const row of rows) for (const value of row.slice(3)) authorityDigest(value);
  assert(rows.slice(0, -1).every(r => r[3] === rows[0][3] && r[4] === rows[0][3]), "original parser/inventory authority changed");
  const failed = rows.at(-1);
  assert(failed.length === 5 && failed[0] === "ait.semantic.init" && failed[1] === "fail" && failed[2] === "1");
  assert.equal(c.pinned(path.join(directory, "first-failure.tsv")).toString().trim(), [failed[0], ...failed.slice(2)].join("\t"));
  assert.equal(c.pinned(path.join(directory, ".initial-authority-digest")).toString().trim(), rows[0][3]);
  assert.equal(c.pinned(path.join(directory, "recovery.tsv")).toString().trim(), "ait.semantic.init\tpass");
  assert.equal(c.pinned(path.join(directory, "ait.semantic.init.argv")).toString(), `${c.input.executables.ait.path}\ninit\n--json\n`);
  c.pinned(path.join(directory, "ait.semantic.init.stdout")); c.pinned(path.join(directory, "ait.semantic.init.stderr"));
  const saved = JSON.parse(c.pinned(path.join(directory, "direct-run.config-original.json")));
  const baseline = JSON.parse(c.pinned(path.join(c.originalRoot, "web-admission/evidence/starting-config.json")));
  for (const key of ["workflow_default_scope", "agent_teams", "sprint", "default_author_mode", "default_model", "task_review"]) assert.deepEqual(saved[key], baseline[key]);
  assert(!existsSync(path.join(directory, "summary.json")), "original failed run must not be relabelled");
  for (const id of prefix) claim(id, "original WT execution");
  for (const [command, kind] of matrix) if (kind === "leaf" && command !== "init") {
    if (c.applicability && affectedCommand(command.split(" "))) continue;
    assert(reviewedLeaves.has(command), `command is outside the reviewed assertion disposition: ${command}`);
    claim(`ait.semantic.${command.replaceAll(" ", ".")}`, "reviewed Q/E assertion equivalence");
  }
  for (const id of ["native.current.formal-finish", "remote.lifecycle.preflight", "remote.lifecycle.select-local-history",
    "remote.lifecycle.workflow-ready", "remote.lifecycle.workflow-finish", "remote.lifecycle.verify-land"]) claim(id, "reviewed same-bytes Native execution");
  return claim;
}

export function validatePriorComposition(c, matrix) {
  priorCoverage(retainedContext(c), matrix, new Map());
}

function recordedArgv(file) {
  const lines = regular(file).toString().split("\n");
  assert.equal(lines.pop(), "", "recorded argv lacks its original delimiter");
  return lines;
}
function capture(directory, name) {
  const argv = recordedArgv(path.join(directory, `${name}.argv`));
  regular(path.join(directory, `${name}.stderr`));
  return { argv, body: json(path.join(directory, `${name}.stdout`)) };
}

// Load declarations only from the SHA-bound WT runner. Replace every execution
// boundary with checks of the original captures; never source its top-level run.
function validateRandomScenarios(repository, directory, cli) {
  const source = regular(path.join(repository, "acceptance/run-randomized-commands.sh")).toString();
  const functions = ["json_string", "leaf_contract", "repository_read_one", "repository_read", "line_snapshot_read",
    "task_change_read", "plan_read", "review_evidence_one", "review_evidence_read", "storage_worktree_one",
    "storage_worktree_read", "boundary_rejection_one", "boundary_rejection", "execute_scenario"];
  const declarations = functions.map(name => {
    const matches = [...source.matchAll(new RegExp(`^awt_${name}\\(\\) \\{\\n[\\s\\S]*?^\\}`, "gm"))];
    assert.equal(matches.length, 1, `missing or ambiguous frozen scenario function: ${name}`);
    return matches[0][0];
  }).join("\n");
  const checked = spawnSync("sh", ["-s", "--", directory, cli], { encoding: "utf8", input: `
set -eu
awt_root=$1
awt_ait_bin=$2
awt_tab=$(printf '\\t')
${declarations}
awt_consume() {
  awt_expected_command=$1; awt_expected_expectation=$2; shift 2
  IFS="$awt_tab" read -r awt_group awt_order awt_id awt_command awt_expectation awt_rc awt_verdict awt_argv awt_stdout awt_stderr <&3 || return 1
  awt_invocations=$((awt_invocations + 1))
  [ "$awt_group" = "$awt_current_group" ] && [ "$awt_order" = "$awt_invocations" ] || return 1
  [ "$awt_id" = "$(printf 'random.%04d.%02d' "$awt_current_group" "$awt_invocations")" ] || return 1
  [ "$awt_command" = "$awt_expected_command" ] && [ "$awt_expectation" = "$awt_expected_expectation" ] || return 1
  printf '%s\\n' "$@" | cmp -s - "$awt_root/$awt_argv" || return 1
  awt_last_stdout=$awt_root/$awt_stdout
}
awt_success_json() {
  awt_filter=$2; awt_label=$1; shift 2
  awt_consume "$awt_label" json-success "$@" || return
  jq -e "$awt_filter" "$awt_last_stdout" >/dev/null 2>&1
}
awt_success_text() { awt_label=$1; shift; awt_consume "$awt_label" text-success "$@"; }
awt_expected_failure() { awt_label=$1; shift; awt_consume "$awt_label" nonzero-fail-closed "$@"; }
exec 3<"$awt_root/commands.tsv"
while IFS="$awt_tab" read -r awt_current_group awt_case_scenario awt_case_candidate awt_case_a awt_case_b awt_case_token; do
  awt_invocations=0
  awt_execute_scenario "$awt_case_scenario" "$awt_case_candidate" "$awt_case_a" "$awt_case_b" "$awt_case_token" || {
    printf 'frozen scenario capture differs: group %s (%s)\\n' "$awt_current_group" "$awt_case_scenario" >&2
    exit 1
  }
done <"$awt_root/schedule.tsv"
if IFS= read -r awt_extra <&3; then exit 1; fi
` });
  assert.equal(checked.status, 0, `random scenario argv or output predicate failed: ${checked.stderr}`);
}
function passedCase(directory, id) {
  assert(!existsSync(path.join(directory, "summary.json")), "bounded cases must not be a partial pass-shaped direct run");
  const rows = regular(path.join(directory, "cases.tsv")).toString().trim().split(/\r?\n/).map(l => l.split("\t"));
  assert.equal(rows.length, 1, "bounded execution repeats unrelated or completed cases");
  assert(rows[0].length === 5 && rows[0][0] === id && rows[0][1] === "pass" && rows[0][2] === "0", `bounded case failed or pending: ${id}`);
  for (const value of rows[0].slice(3)) authorityDigest(value);
}

export function validateFinalWebState(final, input, baseline) {
  const config = json(path.join(final, "final-config.json"));
  for (const key of ["workflow_default_scope", "agent_teams", "sprint", "default_author_mode", "default_model", "task_review"]) assert.deepEqual(config[key], baseline[key], "saved controls were not restored");
  assert.equal(config.repo_name, "ait-web-test"); assert.equal(config.repository_index, 23); assert.equal(config.is_worktree, false);
  assert.equal(config.id_namespace_prefix?.value, "WT");
  const status = json(path.join(final, "final-status.json")), queue = json(path.join(final, "final-queue.json"));
  assert.equal(status.current_line, "main"); assert.equal(status.workspace_dirty, false); assert.equal(status.workspace_changed_count, 0); assert.equal(status.worktree_name, null);
  assert.equal(status.head_snapshot_id, input.web_test.head_snapshot_id);
  assert.equal(status.worktree_hygiene.manual_review_candidate_count, 0); assert.equal(status.worktree_hygiene.stale_count, 0); assert.equal(status.reconciliation.total_finding_count, 0);
  assert.equal(queue.summary.attention_required_count, 0); assert.equal(queue.summary.dirty_worktree_count, 0); assert.equal(queue.remote.reviewer_inbox.count, 0);
  assert.equal(regular(path.join(final, "internal-change-argv.tsv")).length, 0);
}

function verifyGap0(c, directory, matrix) {
  const cli = c.input.executables.ait.path;
  const initial = directory; passedCase(initial, "ait.semantic.init");
  const init = capture(initial, "ait.semantic.init", c.input);
  assert.deepEqual(init.argv, [c.input.executables.ait.path, "init", "--json"]);
  assert.equal(init.body.action, "reinitialized"); assert.equal(init.body.repo_name, "ait-web-test"); assert.deepEqual(init.body.repairs, []);
  assert.equal(init.body.repo_root, realpathSync(c.repository)); assert.equal(init.body.authority_path, path.join(realpathSync(c.repository), ".ait"));
  const baseline = JSON.parse(c.pinned(path.join(c.originalRoot, "web-admission/evidence/starting-config.json")));
  assert.equal(baseline.agent_teams, true); assert.equal(init.body.default_author_mode, baseline.default_author_mode);
  const bootstrap = init.body.agent_team_bootstrap;
  assert.equal(bootstrap?.status, "ready"); assert.equal(bootstrap?.session_readiness, "ready");
  assert.equal(bootstrap?.synchronization?.status, "ready"); assert.equal(bootstrap?.work_started, false);
  assert(typeof bootstrap.session_id === "string" && bootstrap.session_id); assert.equal(bootstrap.target_session_id, bootstrap.session_id);

}

function verifyGap1(c, directory, matrix) {
  const cli = c.input.executables.ait.path;
  const fork = directory; passedCase(fork, "native.current.reviewed-fork");
  const forkCaptures = Object.fromEntries(["original", "", "preserved", "owner", "worktree", "correction"].map(suffix =>
    [suffix, capture(fork, `ait.semantic.change.create${suffix ? "." + suffix : ""}`)]));
  const original = forkCaptures.original.body, created = forkCaptures[""], preserved = forkCaptures.preserved.body;
  const owner = forkCaptures.owner.body, worktree = forkCaptures.worktree.body, correction = forkCaptures.correction.body;
  assert.match(original.snapshot_id, /^SNP-[0-9A-F]{12}$/);
  for (const key of ["parent_snapshot_id", "line_name", "message", "files"]) assert(Object.hasOwn(original, key), "review anchor is incomplete");
  assert(Array.isArray(original.files));
  assert.equal(created.body.status, "draft"); assert.equal(created.body.fork_snapshot_id, original.snapshot_id);
  assert.match(created.body.change_id, /^C-[0-9]{2}$/);
  assert.match(created.body.task_id, /^LWTT-[0-9]+$/); assert(created.body.change_ref.startsWith(created.body.task_id + "/C-"));
  for (const key of ["task_id", "change_ref", "fork_snapshot_id"]) assert.equal(owner[key], created.body[key]);
  assert.equal(worktree.bound_task_id, created.body.task_id); assert.equal(worktree.bound_change_ref, null);
  assert.equal(worktree.repo_root, realpathSync(c.repository));
  const forkName = worktree.name ?? worktree.worktree_name;
  assert(typeof forkName === "string" && forkName);
  for (const file of [worktree.path, worktree.open_path, ...(worktree.alias_path ? [worktree.alias_path] : [])])
    assert(typeof file === "string" && path.isAbsolute(file) && path.resolve(file) === file, "owned worktree path differs");
  assert.equal(worktree.open_path, worktree.alias_path ?? worktree.path, "owned worktree alias contract differs");
  if (existsSync(worktree.path) && existsSync(worktree.open_path))
    assert.equal(realpathSync(worktree.path), realpathSync(worktree.open_path), "owned worktree alias points elsewhere");
  const forkCwd = created.argv[1];
  assert([worktree.path, worktree.open_path].includes(forkCwd), "fork capture uses a foreign worktree path");
  for (const label of ["original", "preserved"]) assert.deepEqual(forkCaptures[label].argv, [cli, "snapshot", "show", original.snapshot_id, "--json"]);
  assert.deepEqual(created.argv, ["awt_native_fork_in_dir", forkCwd, cli, "change", "create", created.body.task_id,
    "--title", "ait-web-test-local-reviewed-correction", "--fork-snapshot", original.snapshot_id, "--local", "--json"]);
  assert.deepEqual(forkCaptures.owner.argv, [cli, "change", "show", created.body.task_id, "--local", "--json"]);
  assert.deepEqual(forkCaptures.worktree.argv, [cli, "worktree", "show", forkName, "--json"]);
  assert.deepEqual(forkCaptures.correction.argv, ["awt_in_dir", forkCwd, cli, "snapshot", "create", created.body.task_id,
    "--message", "ait-web-test-reviewed-fork-correction", "--json", "--full"]);
  assert.equal(correction.task_id, created.body.task_id); assert.equal(correction.change_id, created.body.change_id);
  assert.notEqual(correction.snapshot_id, original.snapshot_id);
  assert.match(correction.snapshot_id, /^SNP-[0-9A-F]{12}$/); assert.equal(correction.parent_snapshot_id, original.snapshot_id);
  for (const key of ["snapshot_id", "parent_snapshot_id", "line_name", "message", "files"]) assert.deepEqual(preserved[key], original[key]);

}

function verifyGap2(c, directory, matrix) {
  const cli = c.input.executables.ait.path;
  const dual = directory; passedCase(dual, "native.current.two-authorities");
  const names = readdirSync(path.join(dual, "native-two-authorities"));
  const capturedFile = label => {
    const files = names.filter(n => new RegExp(`^[0-9]{3}\\.${label}\\.stdout$`).test(n));
    assert.equal(files.length, 1, `missing or ambiguous isolation capture: ${label}`);
    return capture(path.join(dual, "native-two-authorities"), files[0].slice(0, -7));
  };
  const read = label => capturedFile(label).body;
  const expect = (label, args) => assert.deepEqual(capturedFile(label).argv, [cli, ...args], `isolation capture command differs: ${label}`);
  const serverArgv = name => recordedArgv(path.join(dual, "native-two-authorities", `${name}.argv`));
  const sa = serverArgv("server-a"), sb = serverArgv("server-b");
  for (const argv of [sa, sb]) {
    assert(path.isAbsolute(argv[2]));
    assert.deepEqual(argv, [c.input.executables.server.path, "--data", argv[2], "--listen", argv[4], "--init-if-missing", "--defer-ci-admission"]);
  }
  const listenA = sa[4], listenB = sb[4];
  assert.match(listenA, /^127\.0\.0\.1:[0-9]+$/); assert.match(listenB, /^127\.0\.0\.1:[0-9]+$/); assert.notEqual(listenA, listenB);
  assert.notEqual(sa[sa.indexOf("--data") + 1], sb[sb.indexOf("--data") + 1]);
  expect("register-a", ["remote", "add", "origin", `http://${listenA}`, "--default", "--json"]);
  expect("register-b", ["remote", "add", "selected-b", `http://${listenB}`, "--default", "--json"]);
  expect("default-a-on-b-client", ["remote", "add", "origin", `http://${listenA}`, "--default", "--json"]);
  expect("authority-a", ["repo", "show", "--remote", "origin", "--json"]);
  expect("authority-b", ["repo", "show", "--remote", "selected-b", "--json"]);
  const a = read("authority-a"), b = read("authority-b"), ta = read("start-origin"), tb = read("start-selected-b");
  assert(Number.isSafeInteger(a.repository.repository_index) && a.repository.repository_index >= 0 && a.repository.repository_index <= 0xffffffff);
  assert.match(ta.task_id, /^R[A-Z]*T-[0-9]+$/);
  assert.equal(a.repository.repository_index, b.repository.repository_index); assert.equal(ta.task_id, tb.task_id);
  for (const [receipt, remote, intent] of [[ta, "origin", "Native authority A"], [tb, "selected-b", "Native authority B"]]) {
    expect(`start-${remote}`, ["task", "start", "--title", intent, "--intent", intent, "--remote", remote, "--json", "--full"]);
    assert.equal(receipt.change ?? null, null); assert.equal(receipt.change_ref ?? null, null);
    assert(typeof receipt.worktree?.name === "string" && receipt.worktree.name);
    expect(`selected-${remote === "origin" ? "a" : "b"}`, ["task", "show", receipt.task_id, "--remote", remote, "--json"]);
    for (const label of [`cleanup-${remote}`, `cleaned-${remote}`]) {
      expect(label, ["task", label.startsWith("cleanup-") ? "abandon" : "show", receipt.task_id, "--remote", remote, "--json"]);
      assert.equal(read(label).task_id, receipt.task_id);
    }
  }
  assert.equal(read("selected-a").intent, "Native authority A"); assert.equal(read("selected-b").intent, "Native authority B");
  const wa = read("worktree-a"), wb = read("worktree-b"), captured = read("capture-b");
  for (const [label, receipt] of [["worktree-a", ta], ["worktree-b", tb], ["a-unchanged-after-b-capture", ta], ["a-head-after-b-cleanup", ta]]) {
    expect(label, ["worktree", "show", receipt.worktree.name, "--json"]);
    assert.equal(read(label).bound_task_id, receipt.task_id); assert.equal(read(label).bound_change_ref, null); assert.equal(read(label).exists, true);
  }
  for (const label of ["a-still-active", "a-unchanged-by-b-cleanup", "same-id-explicit-a-from-b-client"]) {
    expect(label, ["task", "show", ta.task_id, "--remote", "origin", "--json"]); assert.equal(read(label).task_id, ta.task_id);
  }
  assert.equal(read("selected-a").task_id, ta.task_id); assert.equal(read("selected-b").task_id, tb.task_id);
  expect("capture-b", ["snapshot", "create", tb.task_id, "--message", "Native authority B only", "--json", "--full"]);
  expect("immutable-b-snapshot", ["snapshot", "show", captured.snapshot_id, "--json"]);
  assert.notEqual(wa.repo_root, wb.repo_root);
  assert.deepEqual(wa.capture_authority, { remote_name: "origin", repository_index: a.repository.repository_index });
  assert.deepEqual(wb.capture_authority, { remote_name: "selected-b", repository_index: b.repository.repository_index });
  assert.deepEqual(captured.capture_binding, { contract: "ait.snapshot_capture_binding.v2", repository_index: b.repository.repository_index,
    task_id: tb.task_id, change_ref: null, snapshot_id: captured.snapshot_id, line_name: wb.current_line });
  const snap = read("immutable-b-snapshot"); assert.equal(snap.snapshot_id, captured.snapshot_id); assert.equal(snap.message, "Native authority B only");
  assert.equal(snap.files.filter(f => f.path === "selected-authority.txt").length, 1);
  for (const label of ["a-unchanged-after-b-capture", "a-head-after-b-cleanup"]) assert.equal(read(label).head_snapshot_id, wa.head_snapshot_id);
  for (const label of ["a-still-active", "a-unchanged-by-b-cleanup"]) assert.equal(read(label).status, "active");
  for (const label of ["cleanup-selected-b", "cleaned-selected-b", "cleanup-origin", "cleaned-origin"]) assert.equal(read(label).status, "abandoned");
  assert.equal(read("same-id-explicit-a-from-b-client").intent, read("selected-a").intent);

}

function verifyGap3(c, directory, matrix) {
  const cli = c.input.executables.ait.path;
  const randomRoot = directory, random = json(path.join(randomRoot, "summary.json"));
  assert(!existsSync(path.join(randomRoot, "first-failure.tsv")), "random failure was retained beside a conflicting pass");
  assert.equal(random.contract, "ait-web-test.random-command-run.v1"); assert.equal(random.status, "pass");
  assert.equal(random.mode, "solo_remote"); assert.equal(random.repository_index, 23); assert.equal(random.namespace, "WT");
  assert.equal(random.seed, c.input.execution.random_seed); assert.equal(c.input.execution.random_groups, 3000);
  assert.equal(random.executed_groups, 3000); assert.equal(random.requested_groups, 3000); assert.equal(random.replay_case, null);
  assert.equal(random.retirement_invocations, 0); assert.equal(random.initial_authority_digest, random.final_authority_digest);
  authorityDigest(random.initial_authority_digest);
  const expected = mkdtempSync(path.join(tmpdir(), "ait-stage5-schedule-"));
  let scheduleRows;
  try {
    const manifests = path.join(c.repository, "acceptance/manifests"), schedule = path.join(expected, "schedule.tsv");
    const generated = spawnSync("sh", [path.join(c.repository, "acceptance/generate-random-command-schedule.sh"),
      "--matrix", path.join(manifests, "ait-command-matrix.tsv"), "--exclusions", path.join(manifests, "random-command-exclusions.tsv"),
      "--retired", path.join(manifests, "retired-command-paths.tsv"), "--scenarios", path.join(manifests, "random-command-scenarios.tsv"),
      "--groups", "3000", "--seed", String(c.input.execution.random_seed), "--output", schedule], { encoding: "utf8" });
    assert.equal(generated.status, 0, `schedule cannot be verified: ${generated.stderr}`);
    assert(regular(schedule).equals(regular(path.join(randomRoot, "schedule.tsv"))), "random schedule belongs to another seed or matrix");
    scheduleRows = regular(schedule).toString().trim().split(/\r?\n/).map(line => line.split("\t"));
    assert.equal(random.schedule_cksum, JSON.parse(generated.stdout).schedule_cksum);
  } finally { rmSync(expected, { recursive: true, force: true }); }
  const groups = regular(path.join(randomRoot, "groups.tsv")).toString().trim().split(/\r?\n/).map(l => l.split("\t"));
  assert.equal(groups.length, 3000); assert.equal(new Set(groups.map(r => r[0])).size, 3000);
  assert(groups.every((r, index) => r[0] === String(index + 1)));
  assert(groups.every(r => r.length === 8 && r[3] === "pass" && r[4] === "0" && r[6] === r[7]));
  for (const [index, group] of groups.entries()) {
    authorityDigest(group[6]); assert.equal(group[6], random.initial_authority_digest);
    assert.deepEqual(group.slice(0, 3), scheduleRows[index].slice(0, 3), "executed group differs from the selected schedule");
  }
  const commands = regular(path.join(randomRoot, "commands.tsv")).toString().trim().split(/\r?\n/).map(l => l.split("\t"));
  assert.equal(commands.length, random.command_invocations);
  assert.equal(new Set(commands.map(r => r[2])).size, commands.length);
  const excluded = new Set(regular(path.join(c.repository, "acceptance/manifests/random-command-exclusions.tsv")).toString()
    .split(/\r?\n/).filter(l => l.trim() && !l.startsWith("#")).map(l => l.split("\t")[0]));
  const allowed = new Set(matrix.filter(([command, kind]) => kind === "leaf" && !excluded.has(command)).map(([command]) => command));
  assert.equal(random.allowed_commands, allowed.size);
  for (const row of commands) {
    assert.equal(row.length, 10); assert.equal(row[6], "pass"); assert(!["task land", "change land", "repo retire", "repo restore"].includes(row[3]));
    assert(allowed.has(row[3]), "random invocation is outside the selected command matrix");
    assert.equal(row[7], `${row[2]}.argv`); assert.equal(row[8], `${row[2]}.stdout`); assert.equal(row[9], `${row[2]}.stderr`);
    for (const file of row.slice(7)) { assert.equal(path.basename(file), file); regular(path.join(randomRoot, file)); }
    const argv = recordedArgv(path.join(randomRoot, row[7]));
    assert.equal(argv[0], cli, "random capture candidate differs");
    const startsWith = command => command.split(" ").every((token, index) => argv[index + 1] === token);
    assert(startsWith(row[3]), "random declared command contradicts its recorded argv");
    for (const command of new Set([...excluded, "task land", "change land", "repo retire", "repo restore"]))
      assert(!startsWith(command), "excluded or retired random argv");
    if (row[4] === "nonzero-fail-closed") {
      assert(/^[1-9][0-9]*$/.test(row[5])); assert(regular(path.join(randomRoot, row[8])).length || regular(path.join(randomRoot, row[9])).length);
    } else {
      assert(["text-success", "json-success"].includes(row[4])); assert.equal(row[5], "0");
      assert(regular(path.join(randomRoot, row[8])).length);
      if (row[4] === "json-success") JSON.parse(regular(path.join(randomRoot, row[8])));
    }
  }
  const argvFiles = readdirSync(randomRoot).filter(n => /^random\.[0-9]+\.[0-9]+\.argv$/.test(n));
  assert.equal(argvFiles.length, commands.length);
  const invocations = new Map();
  for (const row of commands) { assert(/^[1-9][0-9]*$/.test(row[0]) && Number(row[0]) <= 3000); invocations.set(row[0], (invocations.get(row[0]) ?? 0) + 1); }
  for (const group of groups) assert.equal(String(invocations.get(group[0])), group[5], "random group lacks its invocation evidence");
  const leafCoverage = new Set();
  for (const [index, group] of groups.entries()) if (group[1] === "leaf-contract") {
    const rows = commands.filter(row => row[0] === group[0]);
    assert.equal(rows.length, 2); assert(rows.every(row => row[3] === group[2]));
    assert.deepEqual(rows.map(row => row[4]), ["text-success", "nonzero-fail-closed"]);
    const tokens = group[2].split(" ");
    assert.deepEqual(regular(path.join(randomRoot, rows[0][7])).toString().trimEnd().split("\n"),
      [c.input.executables.ait.path, ...tokens, ...(group[2] === "help" ? [] : ["--help"])]);
    assert.deepEqual(regular(path.join(randomRoot, rows[1][7])).toString().trimEnd().split("\n"),
      [c.input.executables.ait.path, ...tokens, `--ait-web-test-invalid-${scheduleRows[index][5]}`]);
    leafCoverage.add(group[2]);
  }
  assert.deepEqual(leafCoverage, allowed, "selected random campaign omitted a command leaf");
  const scenarioCounts = {};
  for (const group of groups) scenarioCounts[group[1]] = (scenarioCounts[group[1]] ?? 0) + 1;
  assert.deepEqual(random.scenario_counts, scenarioCounts);
  validateRandomScenarios(c.repository, randomRoot, cli);

}

export function verifyBoundedCase(c, name, directory, matrix, allowPartial = false) {
  const selection = recoveryLocations(c.priorInputPath, c.repository, c.assertionCorrection?.snapshot ?? c.input.components.web_test);
  const evidence = retainedContext(c), fresh = c.applicability && ["ait.semantic.init", "closeout"].includes(name);
  const source = !fresh && selection && ({ "ait.semantic.init": selection.initial, "native.current.two-authorities": selection.dual,
    randomized_3000: selection.collection, "native.current.reviewed-fork": selection.fork, closeout: selection.final }[name]);
  if (source) verifyCaseCopy(c, source, directory);
  if (selection?.fork) validateRuntimeInput(evidence, selection);
  if (source && name === "native.current.reviewed-fork") {
    const setup = json(path.join(directory, "ait.semantic.setup-mutation-task.setup.json"));
    const created = capture(directory, "ait.semantic.change.create").body;
    const worktree = capture(directory, "ait.semantic.change.create.worktree").body;
    assert.equal(setup.repo_name, "ait-web-test"); assert.equal(setup.task_id, created.task_id);
    assert.equal(setup.status, "active"); assert.equal(setup.identity_source, "local"); assert.equal(setup.publication_state, "local_draft");
    assert.equal(setup.worktree.bound_task_id, created.task_id); assert.equal(setup.worktree.bound_change_ref, null);
    assert.equal(setup.worktree.repo_root, realpathSync(c.repository));
    assert.equal(setup.worktree.fork_snapshot_id, evidence.input.components.web_test);
    assert.equal(setup.worktree.head_snapshot_id, evidence.input.components.web_test);
    for (const key of ["path", "open_path"]) assert.equal(setup.worktree[key], worktree[key]);
    const config = json(path.join(directory, "ait.semantic.config-original.json"));
    const baseline = JSON.parse(c.pinned(path.join(c.originalRoot, "web-admission/evidence/starting-config.json")));
    for (const key of ["workflow_default_scope", "agent_teams", "sprint", "default_author_mode", "default_model", "task_review"])
      assert.deepEqual(config[key], baseline[key], "retained fork changed saved controls");
  }
  if (fresh && name === "ait.semantic.init") {
    passedCase(directory, name);
    const row = regular(path.join(directory, "cases.tsv")).toString().trim().split("\t");
    assert.equal(row[3], row[4], "isolated candidate checks changed shared WT authority");
    verifyInstalledTeamCaptures(directory, c.input, c.candidate, c.accepted); return;
  }
  if (name === "ait.semantic.init" && selection) {
    validatePartialInit(evidence, directory);
    assert(allowPartial, "semantic-init admission HOLD: only partial frozen-candidate reinit evidence; binding-preservation requires separate Core candidate integration");
    return;
  }
  if (name === "randomized_3000" && selection) return validateRecoveredRandom(evidence, directory, selection);
  const index = ["ait.semantic.init", "native.current.reviewed-fork", "native.current.two-authorities", "randomized_3000"].indexOf(name);
  if (index >= 0) return [verifyGap0, verifyGap1, verifyGap2, verifyGap3][index](evidence, directory, matrix);
  assert.equal(name, "closeout");
  validateFinalWebState(directory, c.input, JSON.parse(c.pinned(path.join(c.originalRoot, "web-admission/evidence/starting-config.json"))));
  if (fresh) for (const [label, args] of [["final-config", ["config", "show", "--json"]], ["final-status", ["status", "--json", "--full"]],
    ["final-queue", ["queue", "summary", "--remote", "origin", "--json"]]]) {
    assert.deepEqual(recordedArgv(path.join(directory, label + ".argv")), [c.input.executables.ait.path, ...args]);
    assert.equal(regular(path.join(directory, label + ".exit")).toString(), "0\n"); regular(path.join(directory, label + ".stderr"));
    assert(regular(path.join(directory, label + ".stdout")).equals(regular(path.join(directory, label + ".json"))));
  }
}

function validateRuntimeInput(c, selection) {
  for (const file of selection.runtimeFiles) c.pinned(file);
  const file = selection.runtimeFiles.find(file => file.endsWith("/qualification-input.json")), input = JSON.parse(c.pinned(file));
  assert.equal(input.contract, "ait.release.web-admission-input/v1");
  sameProductComponents(input.components, c.input);
  const wt = c.assertionCorrection?.snapshot ?? c.input.components.web_test;
  assert.equal(input.components.web_test, wt); assert.equal(input.web_test.head_snapshot_id, wt);
  assert.equal(input.web_test.acceptance_set_sha256, c.assertionCorrection?.acceptanceSha256 ?? c.input.web_test.acceptance_set_sha256);
  for (const key of ["executables", "qualification", "candidate_authority_sha256", "browser_evidence"]) assert.deepEqual(input[key], c.input[key]);
  assert.deepEqual(input.release, c.input.release);
  assert.deepEqual(input.server, c.input.server);
  assert.deepEqual(input.mutation, c.input.mutation); assert.equal(input.status, "bound");
  for (const key of ["random_seed", "random_groups"]) assert.equal(input.execution[key], c.input.execution[key]);
  for (const key of ["order", "result_labels"]) if (c.input.execution[key]) assert.deepEqual(input.execution[key], c.input.execution[key]);
  assert.equal(input.web_test.repository_index, 23); assert.equal(input.web_test.namespace, "WT");
  assert.deepEqual(input.web_test.entry_gate, { line: "main", dirty: false, changed: 0, is_worktree: false, action_required: false, worktree_candidates: 0 });
  const entry = name => selection.runtimeFiles.find(file => file.endsWith("/evidence/" + name));
  assert.equal(input.web_test.starting_authority_digest, hash(Buffer.from(`${c.pinned(entry("starting-status.json"))}\0${c.pinned(entry("starting-queue.json"))}`)));
  assert.equal(c.pinned(entry("acceptance-set.sha256")).toString(), input.web_test.acceptance_set_sha256 + "\n");
  const status = JSON.parse(c.pinned(entry("starting-status.json"))), queue = JSON.parse(c.pinned(entry("starting-queue.json")));
  assert.equal(status.head_snapshot_id, wt); assert.equal(status.current_line, "main"); assert.equal(status.workspace_dirty, false);
  assert.equal(status.workspace_changed_count, 0); assert.equal(status.worktree_name, null);
  assert.equal(status.worktree_hygiene.manual_review_candidate_count, 0); assert.equal(status.worktree_hygiene.stale_count, 0);
  assert.equal(status.reconciliation.total_finding_count, 0); assert.equal(queue.summary.attention_required_count, 0);
  assert.equal(queue.summary.dirty_worktree_count, 0); assert.equal(queue.remote.reviewer_inbox.count, 0);
  const remote = JSON.parse(c.pinned(entry("remote-repository.json"))).repository;
  assert.equal(remote.repository_index, 23); assert.equal(remote.repository_name, "ait-web-test");
  assert.equal(remote.namespace, "WT"); assert.equal(remote.tombstoned, false);
  const components = JSON.parse(c.pinned(selection.runtimeFiles.find(file => file.endsWith("/web-components.json"))));
  assert.deepEqual(input.components, components);
  const accepted = JSON.parse(c.pinned(selection.runtimeFiles.find(file => file.endsWith("/accepted-input.json"))));
  assert.equal(accepted.contract, "ait.release.accepted-input/v1");
  assert.equal(accepted.web.acceptance_snapshot, wt); assert.equal(accepted.paths.web_test_root, c.repository);
  assert.deepEqual(accepted.component_closeouts.map(row => row.id).sort(), ["core", "node", "python", "runner", "server"]);
  for (const row of accepted.component_closeouts) assert.equal(row.snapshot, c.input.components[row.id]);
  const current = JSON.parse(c.pinned(path.join(c.root, "accepted-input.json")));
  assert.deepEqual(accepted.release, current.release); assert.deepEqual(accepted.component_closeouts, current.component_closeouts);
  for (const key of ["native_root", "core_root", "public_source", "web_test_root"]) assert.equal(accepted.paths[key], current.paths[key]);
  const config = JSON.parse(c.pinned(entry("starting-config.json")));
  const baseline = JSON.parse(c.pinned(path.join(c.originalRoot, "web-admission/evidence/starting-config.json")));
  for (const key of ["workflow_default_scope", "agent_teams", "sprint", "default_author_mode", "default_model", "task_review"])
    assert.deepEqual(config[key], baseline[key], "retained runtime input changed saved controls");
}

function verifyCaseCopy(c, source, directory) {
  c.pinnedTree(source);
  if (source === directory) return;
  const observed = [];
  const walk = (file, relative = "") => {
    const stat = lstatSync(file); assert(!stat.isSymbolicLink(), "adopted case has a symlink");
    if (stat.isDirectory()) for (const name of readdirSync(file).sort()) walk(path.join(file, name), path.join(relative, name));
    else { assert(stat.isFile()); observed.push(relative); assert(regular(file).equals(c.pinned(path.join(source, relative))), `adopted case changed: ${relative}`); }
  };
  walk(directory);
  const expected = Object.keys(c.plan.source.control_files).filter(file => below(file, source)).map(file => path.relative(source, file)).sort();
  assert.deepEqual(observed.sort(), expected, "adopted case inventory differs");
}

function validatePartialInit(c, directory) {
  const cases = regular(path.join(directory, "cases.tsv")).toString().trim().split(/\r?\n/).map(line => line.split("\t"));
  assert.equal(cases.length, 1); assert.equal(cases[0].length, 5);
  assert.equal(cases[0][0], "ait.semantic.init"); assert.equal(cases[0][1], "fail");
  assert(/^[1-9][0-9]*$/.test(cases[0][2]));
  for (const value of cases[0].slice(3)) authorityDigest(value);
  assert.equal(regular(path.join(directory, "first-failure.tsv")).toString().trim(), [cases[0][0], ...cases[0].slice(2)].join("\t"));
  assert(!existsSync(path.join(directory, "summary.json")));
  const init = capture(directory, "ait.semantic.init");
  assert.deepEqual(init.argv, [c.input.executables.ait.path, "init", "--json"]);
  assert.equal(init.body.action, "reinitialized"); assert.equal(init.body.repo_name, "ait-web-test"); assert.deepEqual(init.body.repairs, []);
  assert.equal(init.body.repo_root, realpathSync(c.repository)); assert.equal(init.body.authority_path, path.join(realpathSync(c.repository), ".ait"));
  assert.equal(init.body.agent_team_bootstrap?.work_started, false); assert.equal(init.body.agent_team_bootstrap?.status, "pending");
  assert.equal(init.body.agent_team_bootstrap?.detail, "Calling conversation belongs to another Agent; no adoption attempted");
}

export function composeWebCoverage(c, matrix) {
  const coverage = new Map(), claim = priorCoverage(retainedContext(c), matrix, coverage);
  const cli = c.input.executables.ait.path;
  const baseline = JSON.parse(c.pinned(path.join(c.originalRoot, "web-admission/evidence/starting-config.json")));
  const initial = c.result("ait.semantic.init"), fork = c.result("native.current.reviewed-fork"), dual = c.result("native.current.two-authorities"), randomRoot = c.result("randomized_3000");
  for (const [name, directory] of [["ait.semantic.init", initial], ["native.current.reviewed-fork", fork], ["native.current.two-authorities", dual], ["randomized_3000", randomRoot]])
    verifyBoundedCase(c, name, directory, matrix);
  if (c.applicability) {
    verifyFreshQualification(c);
    for (const [command, kind] of matrix) if (kind === "leaf" && affectedCommand(command.split(" ")))
      claim("ait.semantic." + command.replaceAll(" ", "."), "fresh installed candidate; isolated Core preservation/recovery fixtures");
  } else claim("ait.semantic.init", "genuine WT init assertions");
  claim("native.current.reviewed-fork", "exact reviewed Snapshot and correction");
  claim("native.current.two-authorities", "two live selected authorities with equal IDs");
  claim("randomized_3000", "complete selected deterministic WT coverage");

  const final = c.result("closeout"); verifyBoundedCase(c, "closeout", final, matrix);
  const roots = [initial, fork, dual, randomRoot, final];
  const argvCheck = file => {
    const st = lstatSync(file); assert(!st.isSymbolicLink());
    if (st.isDirectory()) for (const name of readdirSync(file)) argvCheck(path.join(file, name));
    else if (file.endsWith(".argv")) assert(!/[A-Z]+T-[0-9]{4}\/C-[0-9]{2}/.test(regular(file).toString()), "internal Change argv leaked");
  };
  for (const root of roots) argvCheck(root);
  // Re-read native output digests after all predicates, rejecting concurrent
  // writes rather than accepting a mixture of captures.
  for (const name of ["ait.semantic.init", "native.current.reviewed-fork", "native.current.two-authorities", "randomized_3000", "closeout"]) c.result(name);
  claim("final.clean-state", "post-gap current WT state");
  claim("final.solo-local-mode", "post-gap current WT state"); claim("final.solo-remote-mode", "post-gap current WT state");
  const expectedCoverage = new Set(["inventory.ait", "native.current.reviewed-fork", "native.current.two-authorities", "native.current.formal-finish",
    "remote.lifecycle.preflight", "remote.lifecycle.select-local-history", "remote.lifecycle.workflow-ready", "remote.lifecycle.workflow-finish",
    "remote.lifecycle.verify-land", "randomized_3000", "final.clean-state", "final.solo-local-mode", "final.solo-remote-mode"]);
  for (const [command, kind] of matrix) {
    const key = command.replaceAll(" ", "."); expectedCoverage.add(`ait.help.${key}`);
    if (kind === "leaf") { expectedCoverage.add(`ait.invalid.${key}`); expectedCoverage.add(`ait.semantic.${key}`); }
  }
  assert.deepEqual(new Set(coverage.keys()), expectedCoverage, "effective assertion coverage is incomplete or conflicting");
  assert(regular(c.file).equals(c.bytes), "replacement Plan changed during verification");
  for (const file of Object.keys(c.plan.source.control_files)) c.pinned(file);
  c.current?.verify();
  return coverage;
}

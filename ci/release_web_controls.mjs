#!/usr/bin/env node
import assert from "node:assert/strict";
import { existsSync, lstatSync, readFileSync, readdirSync, realpathSync } from "node:fs";
import { createHash } from "node:crypto";
import path from "node:path";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import { recoverReceiptTransaction } from "./release_receipt_transaction.mjs";
import { compositionContext, composeWebCoverage } from "./release_web_composition.mjs";

export function currentControls(config) {
  const scope = config?.workflow_default_scope;
  if (Object.hasOwn(config ?? {}, "workflow_mode") || !scope ||
      JSON.stringify(Object.keys(scope).sort()) !== JSON.stringify(["change", "task", "workflow"]) ||
      typeof config.agent_teams !== "boolean" || !["on", "off"].includes(config.sprint?.value) ||
      config.sprint.enabled !== (config.sprint.value === "on") ||
      typeof config.sprint.source !== "string" || !config.sprint.source) {
    throw new Error("Web admission requires the exact current native scope/team/sprint contract");
  }
  const scopes = {};
  for (const key of ["workflow", "task", "change"]) {
    const value = scope[key];
    if (!["local", "remote"].includes(value?.value) || typeof value.source !== "string" || !value.source) {
      throw new Error(`Web admission has malformed ${key} default scope`);
    }
    scopes[key] = { value: value.value, source: value.source };
  }
  return { workflow_default_scope: scopes, agent_teams: config.agent_teams,
    sprint: { value: config.sprint.value, source: config.sprint.source } };
}

export function unchangedSettings(before, after) {
  assert.deepEqual(currentControls(after), currentControls(before), "Web controls were not restored");
  for (const key of ["default_author_mode", "default_model", "task_review"]) {
    assert.equal(Object.hasOwn(after, key), Object.hasOwn(before, key), `${key} presence changed`);
    assert.deepEqual(after[key], before[key], `${key} was not preserved`);
  }
}

export function validateCurrentDoctor(config, doctor) {
  const controls = currentControls(config);
  if (doctor?.contract !== "ait.web.personal.doctor.v2" || doctor.ok !== true ||
      doctor.repository?.name !== config.repo_name || doctor.repository.repository_index !== config.repository_index ||
      Object.hasOwn(doctor.repository, "workflow_mode")) throw new Error("current candidate requires exact Personal doctor.v2");
  assert.deepEqual(doctor.repository.workflow_default_scope, controls.workflow_default_scope);
  assert.equal(doctor.repository.agent_teams, controls.agent_teams);
  assert.equal(doctor.repository.sprint, controls.sprint.value);
}

// The owner's deferral is specific to this release. Presence of the existing
// Personal executable selects the original full-Web contract.
export function webPersonalInScope(input) {
  const personal = Object.hasOwn(input.executables ?? {}, "personal");
  assert.equal(personal, Object.hasOwn(input.components ?? {}, "personal"), "Personal executable and component must be selected or omitted together");
  if (personal) {
    assert.match(input.components.personal, /^SNP-[0-9A-F]{12}$/);
    assert(input.executables.personal && typeof input.executables.personal === "object");
    assert(typeof input.executables.personal.path === "string" && path.isAbsolute(input.executables.personal.path));
    assert.match(input.executables.personal.sha256, /^[0-9a-f]{64}$/);
  }
  assert(personal || input.release?.version === "1.2.0", "Personal omission is authorized only for the selected 1.2.0 release");
  return personal;
}

// Existing executable selection fixes the scope before any evidence is read.
// The paused companion is omitted only on the owner's stable 1.2.0 path.
export function webCommunityInScope(input) {
  const community = Object.hasOwn(input.executables ?? {}, "community");
  assert.equal(community, Object.hasOwn(input.executables ?? {}, "community_cli"), "Community binaries must be selected or omitted together");
  assert.equal(community, Object.hasOwn(input.components ?? {}, "community"), "Community binaries and source must be selected or omitted together");
  if (community) assert.match(input.components.community, /^SNP-[0-9A-F]{12}$/);
  const personal = webPersonalInScope(input);
  assert(community || (!personal && input.release?.version === "1.2.0" && input.release.channel === "stable"),
    "Community omission is authorized only for the owner-paused stable 1.2.0 scope");
  for (const key of community ? ["community", "community_cli"] : []) {
    const row = input.executables[key];
    assert(row && typeof row.path === "string" && path.isAbsolute(row.path));
    assert.match(row.sha256, /^[0-9a-f]{64}$/);
  }
  return community;
}

export function webCommunityPathsInScope(paths, release, personal) {
  const selected = ["community_root", "community_bin", "community_cli_bin", "chrome"].map(key => Object.hasOwn(paths, key));
  assert(selected.every(Boolean) || selected.every(value => !value), "Community source, binaries and Chrome must be selected or omitted together");
  assert(selected[0] || (!personal && release?.version === "1.2.0" && release.channel === "stable"),
    "Community omission is authorized only for the owner-paused stable 1.2.0 scope");
  return selected[0];
}

export function validateWebOperation(inputPath, input, repository, outputRoot = path.dirname(inputPath), preparedPlan) {
  const personal = webPersonalInScope(input);
  const community = webCommunityInScope(input);
  if (!community) assert.deepEqual(input.browser_evidence, {}, "paused companions must not carry browser evidence");
  const read = file => { const st = lstatSync(file); assert(st.isFile() && !st.isSymbolicLink()); return readFileSync(file); };
  const inputBytes = read(inputPath), inputSha = createHash("sha256").update(inputBytes).digest("hex");
  const composition = compositionContext(inputPath, outputRoot, repository, preparedPlan);
  if (composition) assert.deepEqual(input, composition.input, "composition selected another input");
  for (const step of ["foundation", "solo_local", "solo_remote", "randomized_3000", "closeout"]) {
    const file = path.join(path.dirname(inputPath), "receipts", step + ".json");
    if (!existsSync(file)) continue;
    const receipt = JSON.parse(read(file));
    assert.equal(receipt.contract, "ait.release.web-admission-step/v1"); assert.equal(receipt.step, step); assert.equal(receipt.status, "pass");
    assert.equal(receipt.input_sha256, inputSha, "existing evidence belongs to another immutable input");
    const evidence = read(receipt.evidence.path);
    assert.equal(createHash("sha256").update(evidence).digest("hex"), receipt.evidence.sha256, "existing evidence changed");
    if (repository && ["foundation", "solo_local", "solo_remote"].includes(step)) {
      const validator = composition?.assertionCorrection ? validateWebDirectEvidence : validateWebDirect;
      validator(JSON.parse(evidence), input, { foundation: "inventory", solo_local: "solo-local", solo_remote: "solo-remote" }[step], repository, path.dirname(receipt.evidence.path));
    }
  }
  for (const mode of community ? ["solo_local", "solo_remote"] : []) {
    const file = path.join(path.dirname(inputPath), input.browser_evidence[mode].path);
    if (existsSync(file)) validateWebBrowser(JSON.parse(read(file)), input, mode, repository);
  }
  let root = path.dirname(inputPath);
  while (!existsSync(path.join(root, "conductor-plan.json")) && root !== path.dirname(root)) root = path.dirname(root);
  if (!existsSync(path.join(root, "conductor-plan.json"))) {
    assert(personal, "retained input requires its existing conductor operation/digest binding");
    return; // Historical full-Web standalone inputs keep their existing path.
  }
  const planBytes = read(path.join(root, "conductor-plan.json")), plan = JSON.parse(planBytes);
  const state = JSON.parse(read(path.join(root, "conductor-state.json")));
  assert.equal(plan.contract, "ait.release.conductor-plan/v1");
  assert.equal(realpathSync(plan.records_root), realpathSync(root));
  assert.equal(state.contract, "ait.release.conductor-state/v1");
  assert.equal(state.plan_sha256, createHash("sha256").update(planBytes).digest("hex"));
  assert.deepEqual(state.release, plan.release);
  const phase = plan.phases.find(row => row.id === "web_admission");
  const action = phase?.actions.find(row => row.id === "bind-web-admission-input");
  const recorded = state.phases.find(row => row.id === "web_admission")?.actions.find(row => row.id === action?.id);
  assert.equal(recorded?.status, "complete", "original qualification input is not bound by the operation");
  assert.equal(action.type, "command");
  assert(action.outputs.includes(inputPath));
  const mode = lstatSync(inputPath).mode & 0o777;
  const digest = createHash("sha256").update(`f\0.\0${mode}\0`).update(inputBytes).digest("hex");
  assert.equal(recorded.outputs?.[inputPath], digest, "original qualification input digest changed");
  const acceptedPath = path.join(root, "accepted-input.json");
  if (plan.source?.control_files?.[acceptedPath]) {
    const accepted = JSON.parse(read(acceptedPath));
    if (composition?.assertionCorrection || path.dirname(accepted.paths.web_components) !== root) {
      // A fresh qualified binding must come from the native producer. A
      // pass-shaped state file alone cannot stand in for that transaction.
      assert.equal(community, false);
      const checkpoint = JSON.parse(read(path.join(root, "conductor-checkpoints", `${action.id}.json`)));
      assert.equal(checkpoint.plan_sha256, state.plan_sha256); assert.deepEqual(checkpoint.action, recorded);
      assert.equal(recoverReceiptTransaction(root, action.id, {
        plan_sha256: state.plan_sha256, release: state.release, action, replacements: {},
      }), true, "fresh qualification input has no native producer receipt");
      // A reviewed replacement Plan pins its current controls and every reused
      // input. The old Plan stays immutable and cannot admit changed private
      // consumer bytes. Its original input producer is still verified above.
      const status = composition ? { status: 0 } : spawnSync(process.execPath, [path.join(accepted.paths.native_root, "ci/release_conductor.mjs"), "status",
        "--plan", path.join(root, "conductor-plan.json"), "--state", path.join(root, "conductor-state.json")],
      { encoding: "utf8", env: { ...process.env, AIT_RELEASE_AIT_BIN: process.env.AIT_RELEASE_AIT_BIN ?? input.executables.ait.path } });
      assert.equal(status.status, 0, `fresh qualification operation is no longer admitted: ${status.stderr}`);
    }
  }
  for (const key of ["version", "channel"]) assert.equal(input.release[key], plan.release[key]);
  const argument = key => { const positions = action.argv.flatMap((arg, index) => arg === key ? [index] : []); assert.equal(positions.length, 1); return action.argv[positions[0] + 1]; };
  const candidate = JSON.parse(read(argument("--candidate"))), components = JSON.parse(read(argument("--components")));
  assert.equal(candidate.contract, "ait.release.operator.pre-tag-candidate-binding/v1");
  assert.equal(candidate.status, "ready_for_immutable_tag");
  assert.deepEqual(input.components, components);
  for (const key of ["id", "version", "channel"]) assert.equal(input.release[key], candidate.release[key]);
  const inputHasPrior = Object.hasOwn(input.release, "prior_version");
  const candidateHasPrior = Object.hasOwn(candidate.release, "prior_version");
  assert.equal(inputHasPrior, candidateHasPrior, "candidate and derived input must carry or omit prior_version together");
  const prior = candidate.candidate_authority?.record?.prior_release?.version;
  assert.equal(typeof prior, "string", "candidate authority must bind the prior release");
  assert.equal(prior, plan.release.prior_version, "candidate authority prior release differs from the immutable Plan");
  if (inputHasPrior) {
    assert.equal(input.release.prior_version, prior, "input prior version differs from the candidate authority");
    assert.equal(candidate.release.prior_version, prior, "candidate prior version differs from its authority");
  }
  // The original frozen candidate/input may both omit the already required
  // field. Its immutable authority still supplies the exact Plan value; the
  // operation must retain the conductor's stable patch/minor transition.
  const before = /^(\d+)\.(\d+)\.(\d+)$/.exec(prior);
  const after = /^(\d+)\.(\d+)\.(\d+)$/.exec(candidate.release.version);
  assert.equal(candidate.release.channel, "stable");
  assert(before && after, "Web prior release must use the stable transition");
  const old = before.slice(1).map(Number), next = after.slice(1).map(Number);
  assert(next[0] === old[0] && ((next[1] === old[1] && next[2] === old[2] + 1) ||
    (next[1] === old[1] + 1 && next[2] === 0)), "Web prior release must use one stable patch or minor advance");
  assert.equal(input.candidate_authority_sha256, candidate.candidate_authority?.sha256);
  assert.deepEqual(input.qualification, candidate.qualification);
  return composition;
}

export function validateComposedWebInputs(inputPath, input, repository, outputRoot, preparedPlan) {
  const context = validateWebOperation(inputPath, input, repository, outputRoot, preparedPlan);
  assert(context, "bounded coverage requires its replacement control binding");
  assert.equal(webAcceptanceTree(path.join(repository, "acceptance")).sha256, context.assertionCorrection?.acceptanceSha256 ?? input.web_test.acceptance_set_sha256,
    "the bound acceptance tree changed");
  const foundation = path.join(context.originalRoot, "web-admission/receipts/foundation.json");
  const receipt = JSON.parse(context.pinned(foundation));
  context.pinnedTree(path.dirname(receipt.evidence.path));
  assert.equal(receipt.contract, "ait.release.web-admission-step/v1");
  assert.equal(receipt.step, "foundation"); assert.equal(receipt.status, "pass");
  assert.equal(receipt.input_sha256, createHash("sha256").update(context.pinned(context.priorInputPath)).digest("hex"));
  const evidence = context.pinned(receipt.evidence.path);
  assert.equal(receipt.evidence.sha256, createHash("sha256").update(evidence).digest("hex"));
  if (context.assertionCorrection) validateWebDirectEvidence(JSON.parse(evidence), context.priorInput, "inventory", repository, path.dirname(receipt.evidence.path));
  for (const step of ["foundation", "solo_local", "solo_remote", "randomized_3000"]) assert(!existsSync(path.join(outputRoot, "receipts", `${step}.json`)),
    "bounded composition must not carry duplicate or synthetic step receipts");
  const matrix = readFileSync(path.join(repository, "acceptance/manifests/ait-command-matrix.tsv"), "utf8")
    .split(/\r?\n/).filter(line => line.trim() && !line.startsWith("#")).map(line => line.split("\t"));
  assert(matrix.every(row => row.length === 6 && ["leaf", "namespace"].includes(row[1])), "bound command matrix is malformed");
  return { context, matrix };
}

export function validateComposedWebCoverage(inputPath, input, repository, outputRoot) {
  const { context, matrix } = validateComposedWebInputs(inputPath, input, repository, outputRoot);
  return composeWebCoverage(context, matrix);
}

export function webAcceptanceTree(root) {
  const hash = createHash("sha256"), files = [];
  function walk(directory, relative = "") {
    for (const name of readdirSync(directory).sort()) {
      const file = path.join(directory, name), next = path.join(relative, name), stat = lstatSync(file);
      assert(!stat.isSymbolicLink(), `acceptance tree contains a symlink: ${next}`);
      if (stat.isDirectory()) walk(file, next);
      else { assert(stat.isFile(), `acceptance tree contains a special file: ${next}`); files.push({ file, relative: next }); }
    }
  }
  walk(root);
  for (const row of files) hash.update(row.relative).update("\0").update(readFileSync(row.file)).update("\0");
  return { sha256: hash.digest("hex"), files };
}

export function validateBrowserRuntimeHead(runtimeHead, input, repository) {
  const original = input.web_test?.head_snapshot_id;
  assert.match(original ?? "", /^SNP-[0-9A-F]{12}$/);
  assert.match(runtimeHead ?? "", /^SNP-[0-9A-F]{12}$/);
  assert.equal(original.length, 16); assert.equal(runtimeHead.length, 16);
  assert.equal(input.components?.web_test, original, "original Web source identity differs from its input");
  if (runtimeHead === original) return;
  assert(webPersonalInScope(input), "runtime head equivalence requires the full Personal lifecycle scope");
  assert(typeof repository === "string" && path.isAbsolute(repository), "runtime head proof requires the bound Repository");
  const reader = input.executables?.ait;
  const readerStat = lstatSync(reader.path);
  assert(readerStat.isFile() && !readerStat.isSymbolicLink(), "runtime head reader must be the original regular program");
  assert.equal(createHash("sha256").update(readFileSync(reader.path)).digest("hex"), reader.sha256, "runtime head reader changed");
  const ancestor = spawnSync(reader.path, ["snapshot", "is-ancestor", original, runtimeHead], { cwd: repository, encoding: "utf8" });
  assert.equal(ancestor.status, 0, "runtime head is not a proven descendant of the original source");
  const result = spawnSync(reader.path, ["snapshot", "diff", original, runtimeHead, "--json"],
    { cwd: repository, encoding: "utf8", maxBuffer: 32 * 1024 * 1024 });
  assert.equal(result.status, 0, "complete runtime source comparison is unavailable");
  const difference = JSON.parse(result.stdout);
  assert.equal(difference.old_snapshot_id, original); assert.equal(difference.new_snapshot_id, runtimeHead);
  for (const field of ["files", "mode_changed", "added", "deleted", "modified"]) {
    assert.deepEqual(difference[field], [], `runtime source ${field} differs from the original frozen input`);
  }
}

export function validateWebBrowser(value, input, mode, repository) {
  assert(webCommunityInScope(input), "paused Community has no browser qualification");
  const personal = webPersonalInScope(input);
  assert.equal(value.contract, "ait-web-test.browser-evidence.v1");
  assert.equal(value.mode, mode);
  validateBrowserRuntimeHead(value.repository?.head_snapshot_id, input, repository);
  for (const [key, expected] of Object.entries({ index: 23, name: "ait-web-test", namespace: "WT", line: "main", workspace: "clean" })) assert.equal(value.repository[key], expected);
  for (const key of personal ? ["personal", "community"] : ["community"]) {
    const row = value[key]; assert(row, `${key} browser evidence is missing`);
    assert.equal(row.binary_sha256, input.executables[key].sha256);
    for (const field of ["browser_desktop", "browser_mobile-390x844", "browser_console"]) assert.equal(row[field], "pass");
    for (const field of ["console_warning_or_error_count", "failed_request_count", "external_request_count"]) assert.equal(row[field], 0);
    assert.equal(row.responsive?.document_horizontal_overflow, false);
    assert.equal(row.responsive.desktop_max_overflow_px, 0); assert.equal(row.responsive.mobile_max_overflow_px, 0);
  }
  if (!personal) assert(!Object.hasOwn(value, "personal"), "deferred Personal must not carry a synthetic pass");
  assert.equal(value.community.companion_binary_sha256, input.executables.community_cli.sha256);
  const catalog = value.community.catalog;
  assert.equal(catalog?.definition_count, 25);
  for (const key of ["enabled_count", "buttons", "forms", "executable_action_links"]) assert.equal(catalog[key], 0);
  for (const key of ["filesystem_path_leak", "repository_identity_leak", "upstream_server_url_leak"]) assert.equal(catalog[key], false);
  assert.deepEqual(value.community.routes, ["/", "/actions"]);
}

export function validateWebDirect(value, input, phase, repository, resultDirectory) {
  assert.equal(webAcceptanceTree(path.join(repository, "acceptance")).sha256, input.web_test.acceptance_set_sha256,
    "the bound acceptance tree changed");
  return validateWebDirectEvidence(value, input, phase, repository, resultDirectory);
}

function validateWebDirectEvidence(value, input, phase, repository, resultDirectory) {
  const personal = webPersonalInScope(input);
  const community = webCommunityInScope(input);
  assert.equal(value.contract, "ait-web-test.direct-run.v1"); assert.equal(value.status, "pass"); assert.equal(value.phase, phase);
  assert.equal(value.mode, phase === "solo-local" ? "solo_local" : "solo_remote");
  // WT uses POSIX cksum fingerprints here; the acceptance tree above remains SHA-256 bound.
  for (const digest of [value.initial_authority_digest, value.final_authority_digest]) {
    const fields = typeof digest === "string" && /^(0|[1-9][0-9]{0,9}):(0|[1-9][0-9]{0,19})$/.exec(digest);
    assert(fields && fields[0] === digest && BigInt(fields[1]) <= 0xffffffffn &&
      BigInt(fields[2]) <= 0xffffffffffffffffn, "authority digest must be canonical POSIX checksum:byte-count");
  }
  if (phase === "inventory") assert.equal(value.initial_authority_digest, value.final_authority_digest);
  const count = file => readFileSync(path.join(repository, "acceptance/manifests", file), "utf8")
    .split(/\r?\n/).filter(line => line.trim() && !line.startsWith("#")).length;
  assert.equal(value.command_nodes, count("ait-command-matrix.tsv"));
  assert.equal(value.community_surfaces, community ? count("community-surfaces.tsv") : 0);
  assert.equal(value.personal_surfaces, personal || (community && phase === "inventory") ? count("personal-surfaces.tsv") : 0);
  if (resultDirectory) {
    const cases = path.join(resultDirectory, "cases.tsv"), stat = lstatSync(cases);
    assert(stat.isFile() && !stat.isSymbolicLink(), "direct cases must be original regular output");
    const rows = readFileSync(cases, "utf8").trim().split(/\r?\n/).map(line => line.split("\t"));
    // WT appends formal Reviewer HOLDs and the same case's later completion.
    // Preserve that journal; only its final, complete success resolves a HOLD.
    const formal = !community && phase === "solo-remote"
      ? rows.findLast(row => row[0] === "native.current.formal-finish") : null;
    const formalCompleted = formal?.length === 5 && formal[1] === "pass" && formal[2] === "0";
    assert(rows.length > 0 && rows.every(row => row[0] && (row[1] === "pass" ||
      (formalCompleted && row[0] === formal[0] && row[1] === "fail" && row.length === 5 && row[2] === "1"))),
    "direct execution contains missing or failed cases");
    if (!personal && phase !== "inventory") assert(!rows.some(row => row[0].startsWith("personal.")), "deferred Personal must not carry pass rows");
    if (!community) assert(!rows.some(row => row[0].startsWith("community.")), "paused Community must not carry pass rows");
    if (phase === "inventory") {
      for (const id of ["inventory.ait", ...(community ? ["inventory.products"] : []), "inventory.authority-unchanged"]) assert(rows.some(row => row[0] === id));
    } else if (!community) {
      // Reuse the exact WT command matrix and its existing Native execution
      // rows. Community UI/protocol rows certify no underlying AIT behavior.
      const passed = id => assert(rows.some(row => row[0] === id), `retained native execution missing: ${id}`);
      const matrix = readFileSync(path.join(repository, "acceptance/manifests/ait-command-matrix.tsv"), "utf8")
        .split(/\r?\n/).filter(line => line.trim() && !line.startsWith("#")).map(line => line.split("\t"));
      for (const [command, kind] of matrix) {
        const key = command.replaceAll(" ", ".");
        passed(`ait.help.${key}`);
        if (kind === "leaf") { passed(`ait.invalid.${key}`); passed(`ait.semantic.${key}`); }
      }
      for (const id of ["inventory.ait", "native.current.reviewed-fork", `final.${phase}-mode`,
        ...(phase === "solo-remote" ? ["native.current.two-authorities", "native.current.formal-finish",
          "remote.lifecycle.workflow-finish", "remote.lifecycle.verify-land"] : [])]) passed(id);
      const baseline = path.join(resultDirectory, "direct-run.config-original.json");
      assert(lstatSync(baseline).isFile() && !lstatSync(baseline).isSymbolicLink());
      currentControls(JSON.parse(readFileSync(baseline, "utf8")));
    } else {
      // Reuse WT's pinned, read-only coverage checker; it owns all native,
      // Community and security row requirements.
      const checked = spawnSync("sh", [path.join(repository, "acceptance/check-mode-coverage.sh"), value.mode, cases],
        { encoding: "utf8", env: { ...process.env, AIT_WEB_TEST_PERSONAL_BIN: personal ? input.executables.personal.path : "" } });
      assert.equal(checked.status, 0, `direct coverage remains HOLD: ${checked.stderr}`);
    }
  }
}

export function renderedControls(definitions, config) {
  const controls = currentControls(config);
  for (const [family, label] of [["workflow", "Default workflow scope"], ["task", "Default Task scope"], ["change", "Default Change scope"]]) {
    const scope = controls.workflow_default_scope[family];
    if (definitions[label] !== `${scope.value} (${scope.source})`) return false;
  }
  return definitions["Agent Teams"] === (controls.agent_teams ? "on" : "off") &&
    definitions.Sprint === `${controls.sprint.value} (${controls.sprint.source})`;
}

export function explicitAuthority(scope, remote) {
  if (scope === "local" && remote === undefined) return { scope, remote: null };
  if (scope === "remote" && typeof remote === "string" && remote === remote.trim() && remote.length > 0 &&
      remote.length <= 256 && !/[\x00-\x1f\x7f/:?@#\\]/.test(remote)) return { scope, remote };
  throw new Error("Browser qualification requires an explicit Local or named Remote authority");
}

export function authorityArgv(authority) {
  const checked = explicitAuthority(authority.scope, authority.remote ?? undefined);
  return checked.scope === "local" ? ["--local"] : ["--remote", checked.remote];
}

export function validateCaptureAuthority(worktree, authority, repositoryIndex) {
  authorityArgv(authority);
  if (!Object.hasOwn(worktree ?? {}, "capture_authority")) throw new Error("Current native capture-authority read is unavailable");
  const proof = worktree.capture_authority;
  if (authority.scope === "local") {
    if (proof !== null) throw new Error("Local capture cannot use a Remote-bound worktree");
    return;
  }
  if (!Number.isSafeInteger(repositoryIndex) || repositoryIndex < 0 || repositoryIndex > 0xffffffff ||
      !proof || JSON.stringify(Object.keys(proof).sort()) !== JSON.stringify(["remote_name", "repository_index"]) ||
      proof.remote_name !== authority.remote || proof.repository_index !== repositoryIndex) {
    throw new Error(`Worktree capture authority differs: expected ${authority.remote}/Repository ${repositoryIndex}; observed ${proof?.remote_name ?? "null"}/Repository ${proof?.repository_index ?? "null"}`);
  }
}

export function scopedReadUrl(base, route, authority) {
  authorityArgv(authority);
  const origin = new URL(base);
  if (typeof route !== "string" || !route.startsWith("/") || route.startsWith("//") || /[\\\x00-\x20]/.test(route)) {
    throw new Error("Personal read route must be same-origin and relative");
  }
  const url = new URL(route, origin);
  if (url.origin !== origin.origin || url.hash || url.searchParams.has("scope") || url.searchParams.has("remote")) {
    throw new Error("Personal read route already has ambiguous authority");
  }
  url.searchParams.set("scope", authority.scope);
  if (authority.scope === "remote") url.searchParams.set("remote", authority.remote);
  return url.href;
}

export function taskStartRequest(source, intent, authority) {
  authorityArgv(authority);
  if (typeof source !== "string" || source.split("#").length !== 2 || source.split("#").some(part => !part) ||
      typeof intent !== "string" || !intent.trim()) throw new Error("Browser Task start requires an exact Plan source and intent");
  return { source, intent, scope: authority.scope, ...(authority.scope === "remote" ? { remote: authority.remote } : {}) };
}

export function taskStartInspectLocation(location, taskId, authority) {
  authorityArgv(authority);
  if (!/^[A-Z]+T-[0-9]+$/.test(taskId ?? "") || typeof location !== "string" ||
      !location.startsWith("/tasks/") || /[\\\x00-\x20]/.test(location)) throw new Error("Task start has no verified relative Inspect Location");
  const url = new URL(location, "http://personal.invalid");
  let id;
  try { id = decodeURIComponent(url.pathname.slice("/tasks/".length)); } catch { throw new Error("Malformed Inspect Task ID"); }
  const keys = [...url.searchParams.keys()];
  if (url.origin !== "http://personal.invalid" || url.hash || id !== taskId ||
      !taskId.startsWith(authority.scope === "local" ? "L" : "R") ||
      location.split("?")[0] !== `/tasks/${encodeURIComponent(taskId)}` ||
      keys.length !== (authority.scope === "local" ? 1 : 2) || new Set(keys).size !== keys.length ||
      keys.some(key => !["scope", "remote"].includes(key)) || url.searchParams.get("scope") !== authority.scope ||
      (authority.scope === "remote" ? url.searchParams.get("remote") !== authority.remote : url.searchParams.has("remote"))) {
    throw new Error("Inspect Location changed the Task or explicit authority");
  }
  return location;
}

export function taskStartOutcome(http, generation, request) {
  if (!["current", "legacy"].includes(generation)) throw new Error("Task-start generation must be selected before mutation");
  const authority = explicitAuthority(request.scope, request.remote);
  const body = http.body, item = request.source.split("#")[1];
  if (http.status === 409) {
    const reconciliation = body?.reconciliation;
    if (body?.contract !== "ait.web.personal.task-start-error.v1" || body.ok !== false ||
        body.operation !== "task.start" || body.automatic_retry !== false || reconciliation?.matched_count !== 1 ||
        reconciliation.status !== "matched_task" || reconciliation.plan_item_ref !== item) {
      throw new Error("Task-start failure has no unique exact-authority reconciliation; do not retry");
    }
    const taskId = reconciliation.matched_task?.task_id;
    return { created: false, taskId, inspectPath: taskStartInspectLocation(http.location, taskId, authority) };
  }
  if (http.status !== 201 || body?.ok !== true || body.operation !== "task.start" || body.source !== request.source ||
      body.plan_item_ref !== item || body.intent !== request.intent || typeof body.title !== "string" || !body.title ||
      !/^[A-Z]+T-[0-9]+$/.test(body.task_id ?? "") || !body.task_id.startsWith(authority.scope === "local" ? "L" : "R") ||
      body.worktree?.name !== body.task_id.toLowerCase() || body.worktree.current_line !== `feature/${body.task_id.toLowerCase()}` ||
      body.worktree.reused !== false || typeof body.worktree.open_path !== "string" || !/^(?:\/|[A-Za-z]:[\\/])/.test(body.worktree.open_path)) {
    throw new Error("Task-start response is not the exact successful invocation; do not retry");
  }
  if (generation === "current" ? body.contract !== "ait.web.personal.task-start-receipt.v2" ||
      !Object.hasOwn(body, "change_ref") || body.change_ref !== null || body.scope !== authority.scope || body.remote !== authority.remote :
      body.contract !== "ait.web.personal.task-start-receipt.v1" || body.change_ref !== `${body.task_id}/C-01` ||
      Object.hasOwn(body, "scope") || Object.hasOwn(body, "remote")) {
    throw new Error("Task-start response contradicts the candidate-selected contract");
  }
  return { created: true, taskId: body.task_id, inspectPath: taskStartInspectLocation(http.location, body.task_id, authority) };
}

if (process.argv[1] && existsSync(process.argv[1]) && realpathSync(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try {
    const [ait, repository, baseline] = process.argv.slice(2);
    const result = spawnSync(ait, ["config", "show", "--json"], { cwd: repository, encoding: "utf8" });
    if (result.status !== 0) throw new Error("Web configuration readback failed");
    const config = JSON.parse(result.stdout);
    currentControls(config);
    if (baseline) unchangedSettings(JSON.parse(readFileSync(baseline, "utf8")), config);
  } catch (error) { process.stderr.write(error.message + "\n"); process.exit(65); }
}

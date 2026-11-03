// The reviewed disposition selects a finite existing evidence inventory. This
// producer adds only hashes to the conductor's existing control_files map.
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { existsSync, lstatSync, readdirSync } from "node:fs";
import path from "node:path";
import { regular } from "./release_web_composition.mjs";
import { qualificationCarrier, correctedWTIdentity } from "./release_web_recovery_identity.mjs";
import { recoveryLocations } from "./release_web_recovery_inputs.mjs";
import { replacementIdentity } from "./release_web_applicability.mjs";

export function reviewedCompositionLocations(nativeRoot) {
  const file = path.join(nativeRoot, "docs/sprints/release_1_2_0_binding.md"), disposition = regular(file).toString();
  const heading = "Stage 5 assertion disposition — 2026-11-01";
  assert.equal(disposition.split(heading).length, 2, "missing or ambiguous reviewed assertion disposition");
  const section = disposition.slice(disposition.indexOf(heading));
  const locations = {};
  for (const label of ["W", "L", "N", "Q", "E"]) {
    const matches = [...section.matchAll(new RegExp("^- " + label + ": `([^`]+)`", "gm"))];
    assert.equal(matches.length, 1, `missing or ambiguous reviewed ${label} location`);
    assert(path.isAbsolute(matches[0][1])); locations[label] = matches[0][1];
  }
  return { file, section, locations };
}

export function compositionInputFiles(inputPath, nativeRoot, spec) {
  const files = {};
  const add = file => {
    assert(path.isAbsolute(file) && path.resolve(file) === file, "composition input path is not canonical");
    for (let item = file; item !== path.dirname(item); item = path.dirname(item))
      assert(!lstatSync(item).isSymbolicLink(), `composition input has a symlink: ${item}`);
    const bytes = regular(file), sha = createHash("sha256").update(bytes).digest("hex");
    if (files[file]) assert.equal(files[file], sha, "composition input changed during selection");
    files[file] = sha;
    return bytes;
  };
  const read = file => JSON.parse(add(file));
  const tree = directory => {
    const stat = lstatSync(directory);
    assert(stat.isDirectory() && !stat.isSymbolicLink(), "composition evidence directory is not regular");
    for (const name of readdirSync(directory).sort()) {
      const file = path.join(directory, name), st = lstatSync(file);
      assert(!st.isSymbolicLink(), `composition evidence has a symlink: ${file}`);
      if (st.isDirectory()) tree(file); else add(file);
    }
  };
  const argument = (action, flag) => {
    assert.equal(action.argv.filter(value => value === flag).length, 1, `ambiguous composition input ${flag}`);
    const value = action.argv[action.argv.indexOf(flag) + 1];
    assert(typeof value === "string" && path.isAbsolute(value)); return value;
  };
  const input = read(inputPath), root = path.dirname(path.dirname(inputPath));
  const selectedRepository = spec?.values.web_test_root;
  const selectedWT = spec && JSON.parse(regular(spec.values.web_components)).web_test;
  const recovery = selectedRepository && recoveryLocations(inputPath, selectedRepository, selectedWT);
  if (recovery) {
    for (const directory of [recovery.initial, recovery.dual, recovery.collection, recovery.fork, recovery.final, ...recovery.priorRoots].filter(Boolean)) tree(directory);
    for (const file of [...recovery.ownerFiles, ...recovery.runtimeFiles]) add(file);
  }
  if (spec) {
    const accepted = read(spec.values.accepted_input_path);
    const selected = read(spec.values.web_components), applicability = replacementIdentity(accepted, input, selected);
    if (!applicability) {
      const carrier = qualificationCarrier(accepted) ?? path.dirname(spec.values.web_components);
      for (const file of [path.join(carrier, "candidate.json"), path.join(carrier, "pre-rc-qualification.json")]) add(file);
    } else for (const file of ["release_web_applicability.mjs", "release_core_team_cases.mjs", "release_prepublish_verify.mjs"])
      add(path.join(nativeRoot, "ci", file));
    const correction = correctedWTIdentity(accepted, input, applicability ? { ...input.components, web_test: selected.web_test } : selected);
    if (correction) for (const file of Object.keys(correction.files)) add(file);
  }
  const { file: dispositionPath, section, locations } = reviewedCompositionLocations(nativeRoot);
  add(dispositionPath);
  const location = label => locations[label];
  assert.equal(path.join(location("W"), "qualification-input.json"), inputPath);
  assert(section.includes(files[inputPath]), "reviewed disposition selects another input");
  const original = read(path.join(root, "conductor-plan.json"));
  const state = read(path.join(root, "conductor-state.json"));
  const action = id => {
    const rows = original.phases.flatMap(phase => phase.actions).filter(row => row.id === id);
    assert.equal(rows.length, 1, `missing or ambiguous earlier producer: ${id}`); return rows[0];
  };
  for (const id of ["qualify-public-source", "qualify-frozen-candidate", "bind-web-admission-input"]) {
    add(path.join(root, "conductor-checkpoints", id + ".json"));
    const receipt = read(path.join(root, "conductor-receipts", id + ".json"));
    const recorded = state.phases.flatMap(phase => phase.actions).filter(row => row.id === id);
    assert.equal(recorded.length, 1); assert.equal(recorded[0].status, "complete");
    assert.deepEqual(receipt.outputs.map(row => row.path).sort(), Object.keys(recorded[0].outputs).sort());
    assert(action(id).outputs.every(file => Object.hasOwn(recorded[0].outputs, file)));
    for (const output of receipt.outputs) {
      if (lstatSync(output.path).isDirectory()) tree(output.path); else add(output.path);
    }
  }
  const bind = action("bind-web-admission-input");
  add(argument(bind, "--candidate")); add(argument(bind, "--components"));
  const accepted = path.join(root, "accepted-input.json");
  if (existsSync(accepted)) add(accepted);
  for (const executable of Object.values(input.executables)) add(executable.path);
  add(path.join(root, "native-runtime-bin/ait-runner"));
  add(path.join(root, "qualified-public-source/ait-monorepo-source.json"));
  add(path.join(root, "web-admission/evidence/starting-config.json"));
  const foundation = read(path.join(root, "web-admission/receipts/foundation.json"));
  assert.equal(path.dirname(foundation.evidence.path), path.join(root, "web-admission/results/foundation"));
  tree(path.dirname(foundation.evidence.path));
  const local = add(path.join(root, "web-admission/private-logs/solo_local.marker")).toString().trim();
  assert.equal(local, location("L"), "Local marker differs from the reviewed disposition"); tree(local);
  const qLog = location("Q"); assert.equal(path.basename(qLog), "q-logs.zip");
  for (const file of [qLog, path.join(path.dirname(qLog), "q-run.json"), path.join(path.dirname(qLog), "q-carrier.zip")]) add(file);
  const eRows = location("E");
  assert(eRows.endsWith("/rows/github-product-<target>.json"), "reviewed installed evidence location differs");
  add(path.dirname(path.dirname(eRows)) + ".zip");
  const native = location("N"); assert.equal(path.basename(native), "native-scenes");
  for (const name of ["local-finish", "explicit-rebase"]) {
    const scene = path.join(native, name), plan = read(path.join(scene, "conductor-plan.json"));
    const closeouts = plan.phases.flatMap(phase => phase.actions).filter(row => row.type === "ait_closeout");
    assert.equal(closeouts.length, 1, "Native scene has ambiguous closeout identity"); add(closeouts[0].ait_bin);
    add(path.join(scene, "conductor-receipts/component.package-identity.json"));
    add(path.join(scene, "conductor-logs", name === "local-finish" ? "component.ait.finish.log" : "component.ait.finish-recovery.log"));
    if (name === "explicit-rebase") for (const file of ["component.ait.finish.log", "component.ait.finish-interruption.json"])
      add(path.join(scene, "conductor-logs", file));
  }
  for (const file of ["candidate-bin/ait-server", "native-runtime-bin/ait-runner"]) add(path.join(path.dirname(native), file));
  // Selection does not itself admit these bytes. Init reuses the consumer's
  // original producer, artifact, identity and predicate checks before writing.
  for (const [file, sha] of Object.entries(files)) assert.equal(createHash("sha256").update(regular(file)).digest("hex"), sha);
  return files;
}

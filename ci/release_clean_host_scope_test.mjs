#!/usr/bin/env node
import assert from "node:assert/strict";
import { mkdtempSync, mkdirSync, readFileSync, rmSync, symlinkSync, writeFileSync } from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { captureRetiredCommands, verifyManualUpgradeDisposition, requiresChatRetirement, validateRetiredBindingMetadata } from "./release_clean_host_phase.mjs";

const root = path.dirname(path.dirname(fileURLToPath(import.meta.url)));
const family = JSON.parse(readFileSync(path.join(root, "ait-release-family.json")));
assert.deepEqual(family.components.map(row => row.id), ["ait", "ait-server", "ait-runner", "ait-python", "ait-node"]);
assert.equal(family.distributions.some(row => row.components.some(id => ["ait-agent", "ait-agent-worker"].includes(id))), false);
const historical = JSON.parse(readFileSync(path.join(root, "release/families/1.1.0/ait-release-family.json")));
for (const id of ["ait-agent", "ait-agent-worker"]) assert(historical.components.some(row => row.id === id));
const oldBinding = { contract: "ait.language.binding.v1", supported_surfaces: ["ait-core", "ait-agent", "ait-agent-worker"] };
validateRetiredBindingMetadata(oldBinding, "1.1.4");
assert.equal(requiresChatRetirement("1.2.0-rc.1"), true);
validateRetiredBindingMetadata({ ...oldBinding, supported_surfaces: ["ait-core"] }, "1.2.0");
assert.throws(() => validateRetiredBindingMetadata(oldBinding, "1.2.0"));
assert.throws(() => validateRetiredBindingMetadata({ ...oldBinding, supported_surfaces: [] }, "1.2.0"));
assert.throws(() => validateRetiredBindingMetadata({ ...oldBinding, supported_surfaces: ["ait-core"], contract: "another-contract" }, "1.2.0"));
assert.throws(() => requiresChatRetirement("unknown"));
const temporary = mkdtempSync(path.join(os.tmpdir(), "ait-manual-upgrade-test."));
try {
  for (const suffix of ["", ".exe"]) {
    const prior = path.join(temporary, "prior" + (suffix || "unix"));
    const candidate = path.join(temporary, "candidate" + (suffix || "unix"));
    mkdirSync(prior); mkdirSync(candidate);
    for (const id of ["ait-agent", "ait-agent-worker"]) writeFileSync(path.join(prior, id + suffix), "immutable legacy bytes");
    writeFileSync(path.join(prior, "user-data.json"), "retain data");
    for (const id of ["ait", "ait-server", "ait-runner"]) writeFileSync(path.join(candidate, id + suffix), "new selected bytes");
    const baseline = captureRetiredCommands(prior, suffix);
    assert.equal(baseline.length, 2);
    const disposition = verifyManualUpgradeDisposition(prior, candidate, baseline, suffix);
    assert.equal(disposition.scope, "side_by_side_manual_origins");
    assert.equal(disposition.retained_legacy_files.length, 2);
    assert.equal(readFileSync(path.join(prior, "user-data.json"), "utf8"), "retain data");
    assert.throws(() => verifyManualUpgradeDisposition(prior, prior, baseline, suffix));
    const forbidden = path.join(candidate, "ait-agent" + suffix);
    writeFileSync(forbidden, "retired binary");
    assert.throws(() => verifyManualUpgradeDisposition(prior, candidate, baseline, suffix));
    rmSync(forbidden);
    symlinkSync(path.join(temporary, "missing-target"), forbidden);
    assert.throws(() => verifyManualUpgradeDisposition(prior, candidate, baseline, suffix));
    rmSync(forbidden);
    writeFileSync(baseline[0].file, "changed legacy bytes");
    assert.throws(() => verifyManualUpgradeDisposition(prior, candidate, baseline, suffix));
  }
  console.log("release public scope and bounded manual upgrade tests passed");
} finally { rmSync(temporary, { recursive: true, force: true }); }

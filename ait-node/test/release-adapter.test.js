import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import path from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");

test("release adapter declares one targetless npm CLI envelope", async () => {
  const manifest = JSON.parse(
    await readFile(path.join(ROOT, "ait-release.json"), "utf8"),
  );
  assert.equal(manifest.schema, "ait.release.adapter/v1");
  assert.deepEqual(manifest.package, {
    name: "ait-native",
    version: "1.0.0-rc.1",
    description: "Portable command-only npm envelope for the AIT release family",
    license_files: [
      { path: "LICENSE", role: "license" },
      { path: "NOTICE", role: "notice" },
    ],
  });
  assert.equal(manifest.components.length, 1);
  const component = manifest.components[0];
  assert.equal(component.id, "ait-node");
  assert.equal(component.ecosystem, "node");
  assert.deepEqual(component.artifacts, [
    {
      path: "dist/ait-native-1.0.0-rc.1.tgz",
      kind: "npm-cli-envelope",
    },
  ]);
  assert.equal(component.artifacts[0].target, undefined);
  for (const dependency of [
    "bin/ait.mjs",
    "bin/ait-server.mjs",
    "bin/launch.mjs",
    "lib/npm-payload-contract.json",
    "release/npm-payload-package.mjs",
    "scripts/fixture-payloads.mjs",
  ]) {
    assert.equal(component.dependency_files.includes(dependency), true);
  }
  assert.equal(
    component.dependency_files.some((entry) => entry.startsWith("native/")),
    false,
  );

  for (const action of ["test", "build", "smoke"]) {
    const driverAction = action === "test" ? "check" : action;
    assert.deepEqual(component.commands[action], [
      [
        "node",
        "release/release-adapter.mjs",
        driverAction,
        "$AIT_RELEASE_TARGET",
        "$AIT_RELEASE_VERSION",
      ],
    ]);
  }
});

test("release driver stays command-only, portable, and registry-inert", async () => {
  const source = await readFile(
    path.join(ROOT, "release", "release-adapter.mjs"),
    "utf8",
  );
  assert.doesNotMatch(source, /shell\s*:/);
  assert.doesNotMatch(source, /https?:\/\//);
  assert.doesNotMatch(source, /npm\s+publish/i);
  assert.doesNotMatch(source, /\b(fetch|cargo|cmake|gradle|dotnet)\b/i);
  assert.match(source, /--ignore-scripts/);
  assert.match(source, /--offline/);
  assert.match(source, /PORTABLE_TARGET = "portable"/);
  assert.match(source, /payload_package_count: 12/);

  const ci = await readFile(path.join(ROOT, "ci", "run.sh"), "utf8");
  assert.match(ci, /\$repo_root\/ait-release\.json/);
  assert.match(ci, /\$repo_root\/release/);
  assert.match(ci, /\$repo_root\/lib/);
  assert.doesNotMatch(ci, /\$repo_root\/native/);
  assert.match(ci, /release-adapter\.mjs build portable/);
  assert.match(ci, /release-adapter\.mjs smoke portable/);
});

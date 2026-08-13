import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import path from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");

test("release adapter declares the portable envelope and six native addons", async () => {
  const manifest = JSON.parse(
    await readFile(path.join(ROOT, "ait-release.json"), "utf8"),
  );
  assert.equal(manifest.schema, "ait.release.adapter/v1");
  assert.deepEqual(manifest.package, {
    name: "ait-native",
    version: "1.0.0-rc.3",
    description: "Direct Node-API envelope and native addon packages for the Rust-owned AIT runtime",
    license_files: [
      { path: "LICENSE", role: "license" },
      { path: "NOTICE", role: "notice" },
    ],
  });
  assert.equal(manifest.components.length, 1);
  const component = manifest.components[0];
  assert.equal(component.id, "ait-node");
  assert.equal(component.ecosystem, "node");
  assert.equal(component.artifacts.length, 7);
  assert.deepEqual(component.artifacts[0], {
    path: "dist/ait-native-1.0.0-rc.3.tgz",
    kind: "npm-napi-envelope",
  });
  assert.equal(component.artifacts[0].target, undefined);
  assert.deepEqual(
    component.artifacts.slice(1).map((artifact) => artifact.kind),
    Array(6).fill("npm-napi-addon"),
  );
  assert.deepEqual(
    new Set(component.artifacts.slice(1).map((artifact) => artifact.target)),
    new Set([
      "aarch64-apple-darwin",
      "x86_64-apple-darwin",
      "aarch64-unknown-linux-gnu",
      "x86_64-unknown-linux-gnu",
      "aarch64-pc-windows-msvc",
      "x86_64-pc-windows-msvc",
    ]),
  );
  for (const dependency of [
    "ait-external.toml",
    "ait-external.lock",
    "bin/ait.mjs",
    "ci/generate_notice.sh",
    "lib/npm-payload-contract.json",
    "src/runtime.js",
    "src/agent.js",
    "release/npm-payload-package.mjs",
    "release/npm-readme.txt",
    "scripts/native-build.mjs",
    "scripts/npm-command.mjs",
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

test("release tools are registry-inert and package direct addon fixtures", async () => {
  const adapter = await readFile(
    path.join(ROOT, "release", "release-adapter.mjs"),
    "utf8",
  );
  const packager = await readFile(
    path.join(ROOT, "release", "npm-payload-package.mjs"),
    "utf8",
  );
  assert.doesNotMatch(adapter, /npm\.cmd/);
  assert.doesNotMatch(adapter, /createRequire/);
  assert.doesNotMatch(adapter, /const addon = require\(packageRoot\)/);
  assert.match(adapter, /const addon = require\(process\.argv\[1\]\)/);
  assert.match(adapter, /run\(process\.execPath/);
  assert.doesNotMatch(`${adapter}\n${packager}`, /npm\s+publish/i);
  assert.doesNotMatch(`${adapter}\n${packager}`, /https?:\/\//);
  assert.match(adapter, /--ignore-scripts/);
  assert.match(adapter, /--offline/);
  assert.match(adapter, /PORTABLE_TARGET = "portable"/);
  assert.match(adapter, /payload_package_count: 6/);
  assert.match(adapter, /runtime_transport: "direct-napi"/);
  assert.match(adapter, /README\.md/);
  assert.match(adapter, /npm-readme\.txt/);
  assert.doesNotMatch(packager, /ait-core release receipt/);
  assert.match(packager, /ait-node built addon/);

  const npmReadme = await readFile(
    path.join(ROOT, "release", "npm-readme.txt"),
    "utf8",
  );
  assert.match(npmReadme, /direct in-process Node-API binding/);
  assert.match(npmReadme, /does not launch an `ait` executable/);
  assert.match(npmReadme, /`ait-server` is distributed separately/);
  assert.match(npmReadme, /does not use install hooks, downloads/);
  assert.match(packager, /ait\.node\.napi-platform-addon\/v1/);
  assert.match(packager, /ait-node built addon/);

  const ci = await readFile(path.join(ROOT, "ci", "run.sh"), "utf8");
  assert.match(ci, /npm run native:build/);
  assert.match(ci, /release-adapter\.mjs build portable 1\.0\.0-rc\.3/);
  assert.match(ci, /release-adapter\.mjs smoke portable 1\.0\.0-rc\.3/);
});

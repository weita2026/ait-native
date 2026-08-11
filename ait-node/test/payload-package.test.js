import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import {
  copyFile,
  lstat,
  mkdir,
  mkdtemp,
  readFile,
  rm,
  symlink,
  writeFile,
} from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const PACKAGER = path.join(ROOT, "release", "npm-payload-package.mjs");
const NPM = process.platform === "win32" ? "npm.cmd" : "npm";

function sha256(bytes) {
  return createHash("sha256").update(bytes).digest("hex");
}

function run(command, args, cwd = ROOT) {
  return spawnSync(command, args, {
    cwd,
    encoding: "utf8",
    env: {
      ...process.env,
      npm_config_audit: "false",
      npm_config_fund: "false",
      npm_config_update_notifier: "false",
    },
    windowsHide: true,
  });
}

async function hostPayloads() {
  const contract = JSON.parse(
    await readFile(
      path.join(ROOT, "lib", "npm-payload-contract.json"),
      "utf8",
    ),
  );
  const payloads = contract.payloads.filter(
    (payload) =>
      payload.os === process.platform && payload.cpu === process.arch,
  );
  assert.equal(payloads.length, 2);
  return payloads;
}

async function receiptFixture(root, payload) {
  const bundle = path.join(root, `${payload.component}-bundle`);
  const artifactPath = `dist/REL-TEST/components/${payload.component}/native/${path.basename(payload.executable)}`;
  const artifact = path.join(bundle, ...artifactPath.split("/"));
  await mkdir(path.dirname(artifact), { recursive: true });
  await copyFile(process.execPath, artifact);
  const artifactBytes = await readFile(artifact);
  const receipt = {
    contract: "ait.release.adapter.receipt/v1",
    repo_name: payload.source_repository,
    snapshot_id: "SNP-123456789ABC",
    version: payload.version,
    target: payload.target,
    metadata: { package: { version: payload.version } },
    artifacts: [
      {
        component: payload.component,
        ecosystem: "native",
        kind: "native-executable",
        role: "component-artifact",
        target: payload.target,
        path: artifactPath,
        sha256: sha256(artifactBytes),
        size_bytes: artifactBytes.length,
      },
    ],
  };
  const receiptPath = path.join(bundle, "ait-release.receipt.json");
  const licensePath = path.join(bundle, "SOURCE-LICENSE");
  const noticePath = path.join(bundle, "SOURCE-NOTICE");
  await writeFile(receiptPath, `${JSON.stringify(receipt, null, 2)}\n`);
  await writeFile(licensePath, `${payload.license} fixture license material\n`);
  await writeFile(noticePath, `${payload.component} fixture notice\n`);
  return { artifact, artifactPath, bundle, licensePath, noticePath, receipt, receiptPath };
}

function packagerArgs(action, payload, fixture, outputRoot) {
  return [
    PACKAGER,
    action,
    "--component",
    payload.component,
    "--target",
    payload.target,
    "--version",
    payload.version,
    "--receipt",
    fixture.receiptPath,
    "--license",
    fixture.licensePath,
    "--notice",
    fixture.noticePath,
    "--out-dir",
    outputRoot,
  ];
}

test("receipt-owned packager builds independently licensed exact payload packages", async (context) => {
  const root = await mkdtemp(path.join(os.tmpdir(), "ait-node-packager-test-"));
  context.after(async () => {
    await rm(root, { recursive: true, force: true });
  });
  const payloads = await hostPayloads();
  for (const payload of payloads) {
    const fixture = await receiptFixture(root, payload);
    const outputRoot = path.join(root, "output");
    const checked = run(
      process.execPath,
      packagerArgs("check", payload, fixture, outputRoot),
    );
    assert.equal(checked.status, 0, checked.stderr);
    const checkResult = JSON.parse(checked.stdout);
    assert.equal(checkResult.package, `${payload.package}@${payload.version}`);
    assert.equal(checkResult.source_snapshot, "SNP-123456789ABC");

    const built = run(
      process.execPath,
      packagerArgs("build", payload, fixture, outputRoot),
    );
    assert.equal(built.status, 0, built.stderr);
    const buildResult = JSON.parse(built.stdout);
    assert.equal(buildResult.component, payload.component);
    assert.equal(buildResult.target, payload.target);
    assert.equal(buildResult.source_sha256, fixture.receipt.artifacts[0].sha256);
    const tarballEntry = await lstat(buildResult.artifact);
    assert.equal(tarballEntry.isFile(), true);
    assert.equal(tarballEntry.isSymbolicLink(), false);

    const installRoot = path.join(root, `install-${payload.component}`);
    const installed = run(NPM, [
      "install",
      "--ignore-scripts",
      "--offline",
      "--no-audit",
      "--no-fund",
      "--no-save",
      "--prefix",
      installRoot,
      buildResult.artifact,
    ]);
    assert.equal(installed.status, 0, installed.stderr);
    const packageRoot = path.join(
      installRoot,
      "node_modules",
      payload.package,
    );
    const packageJson = JSON.parse(
      await readFile(path.join(packageRoot, "package.json"), "utf8"),
    );
    assert.equal(packageJson.name, payload.package);
    assert.equal(packageJson.version, payload.version);
    assert.equal(packageJson.license, payload.license);
    assert.deepEqual(packageJson.os, [payload.os]);
    assert.deepEqual(packageJson.cpu, [payload.cpu]);
    assert.equal(packageJson.bin, undefined);
    assert.equal(packageJson.main, undefined);
    assert.equal(packageJson.exports, undefined);
    assert.deepEqual(packageJson.files, [
      "bin",
      "provenance.json",
      "LICENSE",
      "NOTICE",
    ]);
    assert.equal(packageJson.aitNativePayload.source_repository, payload.source_repository);

    const provenance = JSON.parse(
      await readFile(path.join(packageRoot, "provenance.json"), "utf8"),
    );
    assert.equal(
      provenance.schema,
      "ait.node.npm-platform-payload-provenance/v1",
    );
    assert.equal(provenance.source_snapshot, "SNP-123456789ABC");
    assert.equal(provenance.source_artifact.path, fixture.artifactPath);
    assert.equal(provenance.source_artifact.sha256, fixture.receipt.artifacts[0].sha256);
    assert.equal(provenance.installed_path, payload.executable);
    assert.equal(provenance.license, payload.license);
    assert.equal(provenance.license_file.path, "LICENSE");
    assert.equal(
      provenance.license_file.sha256,
      sha256(await readFile(fixture.licensePath)),
    );
    assert.equal(provenance.notice_file.path, "NOTICE");
    assert.equal(
      provenance.notice_file.sha256,
      sha256(await readFile(fixture.noticePath)),
    );
    assert.equal(
      provenance.source_receipt.sha256,
      sha256(await readFile(fixture.receiptPath)),
    );
    assert.deepEqual(
      await readFile(path.join(packageRoot, "LICENSE")),
      await readFile(fixture.licensePath),
    );
    assert.deepEqual(
      await readFile(path.join(packageRoot, "NOTICE")),
      await readFile(fixture.noticePath),
    );
    const installedBytes = await readFile(
      path.join(packageRoot, ...payload.executable.split("/")),
    );
    assert.equal(sha256(installedBytes), fixture.receipt.artifacts[0].sha256);
  }
});

test("packager rejects missing NOTICE, digest drift, ambiguous artifacts, and traversal", async (context) => {
  const root = await mkdtemp(path.join(os.tmpdir(), "ait-node-packager-reject-"));
  context.after(async () => {
    await rm(root, { recursive: true, force: true });
  });
  const payload = (await hostPayloads()).find(
    (entry) => entry.component === "ait",
  );
  const fixture = await receiptFixture(root, payload);
  const outputRoot = path.join(root, "output");

  const missingNoticeArgs = packagerArgs(
    "check",
    payload,
    fixture,
    outputRoot,
  );
  missingNoticeArgs.splice(missingNoticeArgs.indexOf("--notice"), 2);
  const missingNotice = run(process.execPath, missingNoticeArgs);
  assert.notEqual(missingNotice.status, 0);
  assert.match(
    missingNotice.stderr,
    /missing required npm payload package option --notice/,
  );

  fixture.receipt.artifacts[0].sha256 = "0".repeat(64);
  await writeFile(
    fixture.receiptPath,
    `${JSON.stringify(fixture.receipt, null, 2)}\n`,
  );
  const digestDrift = run(
    process.execPath,
    packagerArgs("check", payload, fixture, outputRoot),
  );
  assert.notEqual(digestDrift.status, 0);
  assert.match(digestDrift.stderr, /SHA-256 drift/);

  fixture.receipt.artifacts.push({ ...fixture.receipt.artifacts[0] });
  await writeFile(
    fixture.receiptPath,
    `${JSON.stringify(fixture.receipt, null, 2)}\n`,
  );
  const ambiguous = run(
    process.execPath,
    packagerArgs("check", payload, fixture, outputRoot),
  );
  assert.notEqual(ambiguous.status, 0);
  assert.match(ambiguous.stderr, /exactly one matching native executable/);

  fixture.receipt.artifacts = [
    {
      ...fixture.receipt.artifacts[0],
      path: "../outside",
      sha256: "1".repeat(64),
    },
  ];
  await writeFile(
    fixture.receiptPath,
    `${JSON.stringify(fixture.receipt, null, 2)}\n`,
  );
  const traversal = run(
    process.execPath,
    packagerArgs("check", payload, fixture, outputRoot),
  );
  assert.notEqual(traversal.status, 0);
  assert.match(traversal.stderr, /path is unsafe/);
});

test(
  "packager rejects a symlinked receipt artifact",
  { skip: process.platform === "win32" },
  async (context) => {
    const root = await mkdtemp(path.join(os.tmpdir(), "ait-node-packager-link-"));
    context.after(async () => {
      await rm(root, { recursive: true, force: true });
    });
    const payload = (await hostPayloads()).find(
      (entry) => entry.component === "ait-server",
    );
    const fixture = await receiptFixture(root, payload);
    await rm(fixture.artifact);
    await symlink(process.execPath, fixture.artifact);
    const linked = run(
      process.execPath,
      packagerArgs("check", payload, fixture, path.join(root, "output")),
    );
    assert.notEqual(linked.status, 0);
    assert.match(linked.stderr, /must be a regular file/);
  },
);

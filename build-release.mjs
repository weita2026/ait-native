#!/usr/bin/env node

import { spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import {
  chmod,
  copyFile,
  lstat,
  mkdir,
  readdir,
  readFile,
  realpath,
  rm,
  stat,
  writeFile,
} from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = path.dirname(fileURLToPath(import.meta.url));
const FAMILY_MANIFEST = path.join(ROOT, "ait-release-family.json");
const SOURCE_MAPPING = path.join(ROOT, "ait-monorepo-source.json");
const RECEIPT_WORKFLOW = path.join(
  ROOT,
  ".github",
  "workflows",
  "ait-release-component-receipts.yml",
);
const PROMOTION_WORKFLOW = path.join(
  ROOT,
  ".github",
  "workflows",
  "ait-release-protected-promotion.yml",
);
const PUBLISHER_WORKFLOW = path.join(
  ROOT,
  ".github",
  "workflows",
  "pypi-publish.yml",
);
const PROMOTION_VERIFIER = path.join(
  ROOT,
  "ait-core",
  "ci",
  "release_protected_promotion.sh",
);
const ENDPOINT_PREPARER = path.join(ROOT, "ci", "release_endpoint_publication.sh");
const ENDPOINT_REMOTE = path.join(ROOT, "ci", "release_endpoint_remote.sh");
const ENDPOINT_CONFIG = path.join(ROOT, "release", "endpoint-publication.rc1.json");
const OCI_SERVER_DOCKERFILE = path.join(ROOT, "release", "oci", "ait-server.Dockerfile");
const OCI_RUNNER_DOCKERFILE = path.join(ROOT, "release", "oci", "ait-runner.Dockerfile");
const BUILD_ROOT = path.join(ROOT, ".build", "source-release");
const OUTPUT_ROOT = path.join(ROOT, "dist", "source-build");
const EXPECTED_REPOSITORIES = [
  "ait-core",
  "ait-server",
  "ait-runner",
  "ait-python",
  "ait-node",
];
const PUBLIC_SOURCE_IDENTITY = "weita2026/ait-native";
const AIT_OPERATIONAL_DIRECTORIES = new Set([
  ".ait",
  ".ait-external",
  ".ait-runtime",
  ".ait-worktree-links",
]);

function fail(message) {
  throw new Error(message);
}

function sha256(bytes) {
  return createHash("sha256").update(bytes).digest("hex");
}

async function readJson(filePath, label) {
  let value;
  try {
    value = JSON.parse(await readFile(filePath, "utf8"));
  } catch (error) {
    fail(`${label} is unavailable or invalid JSON: ${error.message}`);
  }
  return value;
}

async function regularFile(filePath, label) {
  const entry = await lstat(filePath).catch((error) => {
    fail(`${label} is unavailable: ${error.message}`);
  });
  if (!entry.isFile() || entry.isSymbolicLink()) {
    fail(`${label} must be a regular file: ${filePath}`);
  }
  return entry;
}

async function directory(filePath, label) {
  const entry = await lstat(filePath).catch((error) => {
    fail(`${label} is unavailable: ${error.message}`);
  });
  if (!entry.isDirectory() || entry.isSymbolicLink()) {
    fail(`${label} must be a real directory: ${filePath}`);
  }
}

function exactSet(actual, expected, label) {
  const left = [...actual].sort();
  const right = [...expected].sort();
  if (JSON.stringify(left) !== JSON.stringify(right)) {
    fail(`${label} must be exactly ${right.join(", ")}`);
  }
}

async function validateProtectedWorkflows() {
  await regularFile(RECEIPT_WORKFLOW, "root protected component-receipt workflow");
  const workflow = await readFile(RECEIPT_WORKFLOW, "utf8");
  const exactWorkingDirectory =
    "defaults:\n  run:\n    working-directory: ait-core";
  const exactArtifactPath = "          path: ait-core/release-receipt-matrix.json";
  for (const required of [
    "name: ait release component receipts",
    "workflow_dispatch:",
    "permissions:\n  contents: read",
    exactWorkingDirectory,
    exactArtifactPath,
  ]) {
    if (workflow.split(required).length !== 2) {
      fail(`root protected workflow must contain exactly one ${JSON.stringify(required)}`);
    }
  }
  if (
    workflow.includes("contents: write") ||
    workflow.includes("          path: release-receipt-matrix.json")
  ) {
    fail("root protected workflow contains write authority or an unadapted artifact path");
  }

  await regularFile(PROMOTION_WORKFLOW, "root protected promotion workflow");
  const promotion = await readFile(PROMOTION_WORKFLOW, "utf8");
  for (const required of [
    "name: ait release protected promotion",
    "workflow_dispatch:",
    "permissions:\n  actions: read\n  attestations: write\n  contents: read\n  id-token: write",
    "environment:\n      name: rc-promotion",
    "persist-credentials: false",
    "artifact-ids: ${{ inputs.dossier_artifact_id }}",
    "merge-multiple: true",
    "bash control/ait-core/ci/release_protected_promotion.sh",
    "actions/attest-build-provenance@977bb373ede98d70efdf65b84cb5f73e068dcc2a",
    "actions/upload-artifact@ea165f8d65b6e75b540449e92b4886f43607fa02",
  ]) {
    if (promotion.split(required).length !== 2) {
      fail(`root protected promotion workflow must contain exactly one ${JSON.stringify(required)}`);
    }
  }
  for (const forbidden of [
    "contents: write",
    "packages: write",
    "secrets.",
    "gh release create",
    "npm publish",
    "twine upload",
    "docker push",
    "oras push",
  ]) {
    if (promotion.includes(forbidden)) {
      fail(`root protected promotion workflow contains publication authority ${JSON.stringify(forbidden)}`);
    }
  }

  await regularFile(PUBLISHER_WORKFLOW, "root protected endpoint publisher workflow");
  const publisher = await readFile(PUBLISHER_WORKFLOW, "utf8");
  for (const required of [
    "name: ait release endpoint publication",
    "workflow_dispatch:",
    "actions: read\n  attestations: write\n  contents: write\n  id-token: write\n  packages: write",
    "environment:\n      name: pypi",
    "publish_exact_frozen_bytes:",
    "control/ci/release_endpoint_publication.sh",
    "control/ci/release_endpoint_remote.sh preflight",
    "control/release/endpoint-publication.rc1.json",
    "secrets.AIT_NPM_TOKEN",
    "      - name: Publish npm implementation payloads and command envelope\n        continue-on-error: true",
    "      - name: Complete independent endpoint readback",
    "secrets.AIT_HOMEBREW_DEPLOY_KEY",
    "secrets.AIT_APT_REPO_DEPLOY_KEY",
    "secrets.AIT_APT_SIGNING_KEY_B64",
    "vars.AIT_APT_SIGNING_FINGERPRINT",
    "pypa/gh-action-pypi-publish@dc37677b2e1c63e2034f94d8a5b11f265b73ba33",
    "docker/build-push-action@10e90e3645eae34f1e60eeb005ba3a3d33f178e8",
    "actions/attest-build-provenance@977bb373ede98d70efdf65b84cb5f73e068dcc2a",
    "actions/upload-artifact@ea165f8d65b6e75b540449e92b4886f43607fa02",
    "docker logout ghcr.io",
  ]) {
    if (!publisher.includes(required)) {
      fail(`root endpoint publisher workflow must contain ${JSON.stringify(required)}`);
    }
  }
  if ((publisher.match(/^        continue-on-error: true$/gm) ?? []).length !== 1) {
    fail("only the npm publication step may yield to independent endpoints");
  }
  for (const forbidden of [
    "cargo build",
    "cargo install",
    "maturin build",
    "npm run build",
    "build-release.sh",
    "build-release.ps1",
    "wingetcreate submit",
    "winget-pkgs",
    "--overwrite",
  ]) {
    if (publisher.includes(forbidden)) {
      fail(`root endpoint publisher workflow contains forbidden release behavior ${JSON.stringify(forbidden)}`);
    }
  }

  for (const [filePath, label] of [
    [ENDPOINT_PREPARER, "endpoint publication preparer"],
    [ENDPOINT_REMOTE, "authenticated endpoint publisher"],
    [ENDPOINT_CONFIG, "exact endpoint configuration"],
    [OCI_SERVER_DOCKERFILE, "ait-server OCI recipe"],
    [OCI_RUNNER_DOCKERFILE, "ait-runner OCI recipe"],
  ]) {
    await regularFile(filePath, label);
  }
  const endpointConfig = await readJson(ENDPOINT_CONFIG, "exact endpoint configuration");
  if (
    endpointConfig?.contract !== "ait.release.family.endpoints/v1" ||
    endpointConfig?.release?.id !== "REL-FAM-D84070909C7F5CA9" ||
    endpointConfig?.release?.version !== "1.0.0-rc.1" ||
    endpointConfig?.release?.tag !== "v1.0.0-rc.1" ||
    endpointConfig?.release?.source_commit !== "f9d260a8f7046f82a6c3e271d539dd0bbce7bc14" ||
    endpointConfig?.publisher?.workflow !== "pypi-publish.yml" ||
    endpointConfig?.publisher?.environment !== "pypi" ||
    endpointConfig?.endpoints?.winget?.community_manifest_submission !== false
  ) {
    fail("exact endpoint configuration differs from the protected RC route");
  }
  for (const [filePath, component] of [
    [OCI_SERVER_DOCKERFILE, "ait-server"],
    [OCI_RUNNER_DOCKERFILE, "ait-runner"],
  ]) {
    const dockerfile = await readFile(filePath, "utf8");
    for (const required of [
      "# syntax=docker/dockerfile:1.7@sha256:a57df69d0ea827fb7266491f2813635de6f17269be881f696fbfdf2d83dda33e",
      "FROM docker.io/library/debian:bookworm-slim@sha256:abd67ffcfa541b485a3dff59865ab629aa048a6c613e639d36e7456b0b229241",
      `COPY --chmod=0755 bin/\${TARGETARCH}/${component} /usr/local/bin/${component}`,
      "USER 65532:65532",
      `ENTRYPOINT [\"/usr/local/bin/${component}\"]`,
    ]) {
      if (!dockerfile.includes(required)) {
        fail(`${component} OCI recipe must contain ${JSON.stringify(required)}`);
      }
    }
    if (component === "ait-server" && !dockerfile.includes("AITSERVER_LISTEN=0.0.0.0:8088")) {
      fail("ait-server OCI recipe must bind the explicit container-network listener");
    }
    for (const forbidden of ["apt-get", "cargo", "curl", "wget", "git clone"]) {
      if (dockerfile.includes(forbidden)) {
        fail(`${component} OCI recipe contains build or download behavior ${JSON.stringify(forbidden)}`);
      }
    }
  }

  await regularFile(PROMOTION_VERIFIER, "protected promotion verifier");
  const verifier = await readFile(PROMOTION_VERIFIER, "utf8");
  for (const required of [
    "ait.release.family.protected-promotion/v1",
    "authorized_for_explicit_endpoint_promotion",
    "github_protected_environment",
    "request_explicit_registry_authorization",
    "registry_credentials_loaded: false",
    "registry_write: false",
    "github_release_write: false",
    "artifact_rebuild: false",
  ]) {
    if (!verifier.includes(required)) {
      fail(`protected promotion verifier must contain ${JSON.stringify(required)}`);
    }
  }
}

async function validatePublicReadme() {
  const readmePath = path.join(ROOT, "README.md");
  await regularFile(readmePath, "public agent-first README");
  const readme = await readFile(readmePath, "utf8");
  for (const required of [
    "ait init",
    "AGENTS.md",
    "ait workflow tier --json",
    "ait task start",
    "ait blame",
    "ait plan sync",
    "ait snapshot create",
    "ait task land",
    "does not identify the repository's programming language or project type",
    "does not require a running\n`ait-server`",
    "explicitly\nnon-publishable",
  ]) {
    if (!readme.includes(required)) {
      fail(`public README is missing the agent-first contract: ${JSON.stringify(required)}`);
    }
  }
  for (const forbidden of [
    "mkdir -p docs/sprints",
    "Follow the printed `cd` hint",
  ]) {
    if (readme.includes(forbidden)) {
      fail(`public README teaches a manual workflow step: ${JSON.stringify(forbidden)}`);
    }
  }
}

async function validateOperationalIgnorePolicy() {
  const ignorePath = path.join(ROOT, ".gitignore");
  await regularFile(ignorePath, "public root .gitignore");
  const entries = (await readFile(ignorePath, "utf8"))
    .split(/\r?\n/u)
    .filter((entry) => entry !== "");
  exactSet(
    entries,
    [
      "/.build/",
      "/dist/",
      "**/.ait/",
      "**/.ait-external/",
      "**/.ait-runtime/",
      "**/.ait-worktree-links/",
    ],
    "public operational ignore entries",
  );
}

async function validateGitBytePolicy() {
  const attributesPath = path.join(ROOT, ".gitattributes");
  await regularFile(attributesPath, "public root .gitattributes");
  const policy = await readFile(attributesPath, "utf8");
  if (policy !== "* -text\n") {
    fail("public root .gitattributes must preserve every committed byte");
  }
}

async function validateTrackedSourceTree() {
  const topLevel = spawnSync("git", ["rev-parse", "--show-toplevel"], {
    cwd: ROOT,
    encoding: "utf8",
    stdio: ["ignore", "pipe", "ignore"],
    windowsHide: true,
  });
  if (topLevel.error !== undefined || topLevel.status !== 0) {
    return;
  }
  const checkoutRoot = await realpath(topLevel.stdout.trim());
  if (checkoutRoot !== await realpath(ROOT)) {
    return;
  }
  const result = spawnSync("git", ["ls-files", "--stage", "-z"], {
    cwd: ROOT,
    encoding: "utf8",
    stdio: ["ignore", "pipe", "pipe"],
    windowsHide: true,
  });
  if (result.error !== undefined || result.status !== 0) {
    fail(`public source Git index is unavailable: ${result.stderr.trim()}`);
  }
  for (const row of result.stdout.split("\0").filter((value) => value !== "")) {
    const separator = row.indexOf("\t");
    if (separator === -1) {
      fail("public source Git index entry is malformed");
    }
    const metadata = row.slice(0, separator).split(" ");
    const trackedPath = row.slice(separator + 1);
    const segments = trackedPath.split("/");
    if (metadata[0] === "160000") {
      fail(`public source Git index contains a Gitlink: ${trackedPath}`);
    }
    if (
      segments.some((segment) => AIT_OPERATIONAL_DIRECTORIES.has(segment)) ||
      ((segments[0] === ".build" || segments[0] === "dist") && segments.length > 1) ||
      segments.at(-1) === ".gitmodules"
    ) {
      fail(`public source Git index contains operational content: ${trackedPath}`);
    }
  }
}

async function validateTree(current = ROOT, relative = "") {
  const rows = await readdir(current, { withFileTypes: true });
  rows.sort((left, right) => left.name.localeCompare(right.name, "en"));
  for (const row of rows) {
    const childRelative = relative === "" ? row.name : `${relative}/${row.name}`;
    if (row.isSymbolicLink()) {
      fail(`public source tree contains a symlink: ${childRelative}`);
    }
    if (row.isDirectory()) {
      if (row.name === ".git") {
        if (relative === "") {
          continue;
        }
        fail(`public source tree contains nested Git metadata: ${childRelative}`);
      }
      if (AIT_OPERATIONAL_DIRECTORIES.has(row.name)) {
        continue;
      }
      if (relative === "" && (row.name === ".build" || row.name === "dist")) {
        continue;
      }
      await validateTree(path.join(current, row.name), childRelative);
      continue;
    }
    if (!row.isFile()) {
      fail(`public source tree contains a special file: ${childRelative}`);
    }
    if (row.name === ".gitmodules") {
      fail(`public source tree must not contain ${childRelative}`);
    }
  }
}

async function sourceContentDigest() {
  const files = [];
  async function visit(current, relative = "") {
    const rows = await readdir(current, { withFileTypes: true });
    for (const row of rows) {
      const childRelative = relative === "" ? row.name : `${relative}/${row.name}`;
      if (
        relative === "" &&
        (row.name === ".build" ||
          row.name === "dist" ||
          row.name === ".git" ||
          row.name === "ait-monorepo-source.json")
      ) {
        continue;
      }
      if (row.isDirectory() && AIT_OPERATIONAL_DIRECTORIES.has(row.name)) {
        continue;
      }
      const child = path.join(current, row.name);
      if (row.isDirectory()) {
        await visit(child, childRelative);
      } else if (row.isFile()) {
        if (/[\r\n\t]/u.test(childRelative)) {
          fail(`public source path contains a control character: ${childRelative}`);
        }
        files.push({ absolute: child, relative: childRelative });
      } else {
        fail(`public source digest encountered a symlink or special file: ${childRelative}`);
      }
    }
  }
  await visit(ROOT);
  files.sort((left, right) =>
    Buffer.compare(Buffer.from(left.relative, "utf8"), Buffer.from(right.relative, "utf8")),
  );
  const digest = createHash("sha256");
  for (const file of files) {
    const bytes = await readFile(file.absolute);
    digest.update(`${bytes.length}\t${sha256(bytes)}\t${file.relative}\n`, "utf8");
  }
  return digest.digest("hex");
}

function validatePublicSourceContract(family) {
  if (
    family?.schema !== "ait.release.family/v3" ||
    family?.public_source?.model !== "release-monorepo" ||
    family?.public_source?.identity !== "weita2026/ait-native"
  ) {
    fail("family manifest does not declare the admitted public monorepo contract");
  }
  const subtrees = family.public_source.subtrees;
  if (!Array.isArray(subtrees) || subtrees.length !== 5) {
    fail("family manifest must declare five public source subtrees");
  }
  exactSet(
    subtrees.map((row) => row.source_repository),
    EXPECTED_REPOSITORIES,
    "public source repositories",
  );
  for (const row of subtrees) {
    if (row.path !== row.source_repository) {
      fail(`public subtree path drift for ${row.source_repository}`);
    }
    const expectedTransforms =
      row.source_repository === "ait-runner"
        ? ["runner-core-path/v1"]
        : row.source_repository === "ait-python"
          ? ["python-core-path/v1"]
          : [];
    exactSet(row.transforms, expectedTransforms, `${row.source_repository} transforms`);
  }
  const exactTransforms = [
    {
      id: "runner-core-path/v1",
      source_repository: "ait-runner",
      path: "Cargo.toml",
      from: ".ait-external/ait-core/rust/crates/ait-core",
      to: "../ait-core/rust/crates/ait-core",
    },
    {
      id: "python-core-path/v1",
      source_repository: "ait-python",
      path: "pyproject.toml",
      from: ".ait-external/ait-core/rust/crates/ait-py/Cargo.toml",
      to: "../ait-core/rust/crates/ait-py/Cargo.toml",
    },
  ];
  if (JSON.stringify(family.public_source.transforms) !== JSON.stringify(exactTransforms)) {
    fail("family manifest public source transforms differ from the exact allowlist");
  }
  const github = family.distributions.filter((row) => row.channel === "github");
  if (
    github.length !== 1 ||
    github[0].role !== "product" ||
    github[0].identity !== family.public_source.identity
  ) {
    fail("family manifest must contain one product GitHub monorepo distribution");
  }
  exactSet(
    github[0].components,
    family.components.map((row) => row.id),
    "GitHub component coverage",
  );
  exactSet(github[0].targets, family.targets, "GitHub target coverage");
}

function validateSourceMapping(mapping, family) {
  if (
    mapping?.schema !== "ait.release.monorepo-source/v1" ||
    mapping?.public_source_identity !== family.public_source.identity ||
    mapping?.family_version !== family.family.version ||
    mapping?.family_tag !== family.family.tag ||
    !/^SNP-[0-9A-F]{12}$/.test(mapping?.coordinator_snapshot ?? "") ||
    !/^[0-9a-f]{64}$/.test(mapping?.coordinator_manifest_hash ?? "") ||
    !/^(0|[1-9][0-9]*)$/.test(mapping?.coordinator_created_at ?? "") ||
    mapping?.content_digest_contract !==
      "size-sha256-path/v1; excludes ait-monorepo-source.json" ||
    mapping?.git_commit_created !== false ||
    mapping?.public_publish !== false ||
    !/^[0-9a-f]{64}$/.test(mapping?.content_sha256 ?? "") ||
    !/^[0-9a-f]{64}$/.test(mapping?.product_document_sha256 ?? "") ||
    !Array.isArray(mapping?.subtrees) ||
    mapping.subtrees.length !== 5
  ) {
    fail("ait-monorepo-source.json identity or non-public evidence is invalid");
  }
  exactSet(
    mapping.subtrees.map((row) => row.source_repository),
    EXPECTED_REPOSITORIES,
    "source mapping repositories",
  );
  for (const row of mapping.subtrees) {
    const component = family.components.find(
      (candidate) => candidate.source_repository === row.source_repository,
    );
    if (
      component === undefined ||
      row.path !== row.source_repository ||
      row.source_snapshot !== component.source_snapshot ||
      !/^SNP-[0-9A-F]{12}$/.test(row.source_snapshot ?? "") ||
      !/^[0-9a-f]{64}$/.test(row.source_manifest_hash ?? "") ||
      !/^(0|[1-9][0-9]*)$/.test(row.source_snapshot_created_at ?? "") ||
      row.license !== component.license ||
      !/^[0-9a-f]{64}$/.test(row.source_cache_evidence_sha256 ?? "") ||
      !/^[0-9a-f]{64}$/.test(row.source_content_sha256 ?? "") ||
      !/^[0-9a-f]{64}$/.test(row.exported_content_sha256 ?? "")
    ) {
      fail(`source mapping drift for ${row.source_repository}`);
    }
    exactSet(
      row.components,
      family.components
        .filter((candidate) => candidate.source_repository === row.source_repository)
        .map((candidate) => candidate.id),
      `${row.source_repository} source mapping components`,
    );
    exactSet(
      row.transforms,
      family.public_source.subtrees.find(
        (candidate) => candidate.source_repository === row.source_repository,
      ).transforms,
      `${row.source_repository} source mapping transforms`,
    );
  }
}

function gitHead() {
  const result = spawnSync("git", ["rev-parse", "--verify", "HEAD"], {
    cwd: ROOT,
    encoding: "utf8",
    stdio: ["ignore", "pipe", "pipe"],
    windowsHide: true,
  });
  if (result.error !== undefined || result.status !== 0) {
    fail("public source Git commit is unavailable");
  }
  const commit = result.stdout.trim();
  if (!/^(?:[0-9a-f]{40}|[0-9a-f]{64})$/.test(commit)) {
    fail("public source Git commit has an invalid object identity");
  }
  return commit;
}

async function validateBuildInputs(expectedGitCommit) {
  const nodeMajor = Number.parseInt(process.versions.node.split(".")[0], 10);
  if (!Number.isSafeInteger(nodeMajor) || nodeMajor < 20) {
    fail("the public source build requires Node.js 20 or newer");
  }
  const family = await readJson(FAMILY_MANIFEST, "family manifest");
  const mapping = await readJson(SOURCE_MAPPING, "source mapping");
  validatePublicSourceContract(family);
  validateSourceMapping(mapping, family);
  if (
    mapping.family_manifest_sha256 !== sha256(await readFile(FAMILY_MANIFEST)) ||
    mapping.product_document_sha256 !==
      sha256(await readFile(path.join(ROOT, "docs", "distribution.md")))
  ) {
    fail("family manifest or centralized product document differs from its source mapping");
  }
  for (const repository of EXPECTED_REPOSITORIES) {
    await directory(path.join(ROOT, repository), `${repository} source subtree`);
    await regularFile(path.join(ROOT, repository, "LICENSE"), `${repository} LICENSE`);
    await regularFile(path.join(ROOT, repository, "NOTICE"), `${repository} NOTICE`);
  }
  for (const required of [
    ".github/workflows/ait-release-component-receipts.yml",
    ".github/workflows/ait-release-protected-promotion.yml",
    ".github/workflows/pypi-publish.yml",
    ".gitattributes",
    "README.md",
    "ci/release_endpoint_publication.sh",
    "ci/release_endpoint_remote.sh",
    "release/endpoint-publication.rc1.json",
    "release/oci/ait-server.Dockerfile",
    "release/oci/ait-runner.Dockerfile",
    "ait-core/rust/Cargo.toml",
    "ait-core/ci/release_protected_promotion.sh",
    "ait-server/rust/Cargo.toml",
    "ait-runner/Cargo.toml",
    "ait-python/pyproject.toml",
    "ait-node/package.json",
    "ait-node/release/npm-payload-package.mjs",
    "docs/distribution.md",
  ]) {
    await regularFile(path.join(ROOT, required), `required build input ${required}`);
  }
  await validatePublicReadme();
  await validateGitBytePolicy();
  await validateOperationalIgnorePolicy();
  await validateTrackedSourceTree();
  await validateProtectedWorkflows();
  const runnerManifest = await readFile(path.join(ROOT, "ait-runner", "Cargo.toml"), "utf8");
  const pythonManifest = await readFile(path.join(ROOT, "ait-python", "pyproject.toml"), "utf8");
  if (
    !runnerManifest.includes('path = "../ait-core/rust/crates/ait-core"') ||
    runnerManifest.includes(".ait-external/")
  ) {
    fail("ait-runner does not use the exported sibling ait-core path");
  }
  if (
    !pythonManifest.includes('manifest-path = "../ait-core/rust/crates/ait-py/Cargo.toml"') ||
    pythonManifest.includes(".ait-external/")
  ) {
    fail("ait-python does not use the exported sibling ait-core path");
  }
  const nodePackage = await readJson(path.join(ROOT, "ait-node", "package.json"), "npm envelope");
  if (
    nodePackage?.name !== "ait-native" ||
    JSON.stringify(nodePackage?.exports) !== "{}" ||
    ["preinstall", "install", "postinstall"].some((name) => nodePackage?.scripts?.[name] !== undefined)
  ) {
    fail("npm envelope must remain command-only and free of install hooks");
  }
  await validateTree();
  if (mapping.content_sha256 !== (await sourceContentDigest())) {
    fail("public source content differs from ait-monorepo-source.json");
  }
  let gitCommit;
  if (expectedGitCommit !== undefined) {
    if (!/^(?:[0-9a-f]{40}|[0-9a-f]{64})$/.test(expectedGitCommit)) {
      fail("requested public source Git commit is invalid");
    }
    gitCommit = gitHead();
    if (gitCommit !== expectedGitCommit) {
      fail(`checked-out Git commit ${gitCommit} differs from requested ${expectedGitCommit}`);
    }
  }
  return { family, mapping, gitCommit };
}

function run(command, args, cwd = ROOT, extraEnv = {}) {
  const result = spawnSync(command, args, {
    cwd,
    encoding: "utf8",
    env: {
      ...process.env,
      CARGO_INCREMENTAL: "0",
      npm_config_audit: "false",
      npm_config_fund: "false",
      npm_config_update_notifier: "false",
      ...extraEnv,
    },
    stdio: "inherit",
    windowsHide: true,
  });
  if (result.error !== undefined) {
    throw result.error;
  }
  if (result.status !== 0) {
    fail(`${command} ${args.join(" ")} failed with status ${result.status}`);
  }
}

function hostTarget() {
  const key = `${process.platform}/${process.arch}`;
  const targets = new Map([
    ["darwin/arm64", "aarch64-apple-darwin"],
    ["darwin/x64", "x86_64-apple-darwin"],
    ["linux/arm64", "aarch64-unknown-linux-gnu"],
    ["linux/x64", "x86_64-unknown-linux-gnu"],
    ["win32/arm64", "aarch64-pc-windows-msvc"],
    ["win32/x64", "x86_64-pc-windows-msvc"],
  ]);
  const selected = targets.get(key);
  if (selected === undefined) {
    fail(`unsupported source-build host ${key}`);
  }
  return selected;
}

function executableName(name) {
  return process.platform === "win32" ? `${name}.exe` : name;
}

async function copyExecutable(source, destination) {
  await regularFile(source, `built executable ${source}`);
  await mkdir(path.dirname(destination), { recursive: true });
  await copyFile(source, destination);
  if (process.platform !== "win32") {
    await chmod(destination, 0o755);
  }
}

async function artifactRow(component, target, receiptRoot, artifactPath) {
  const bytes = await readFile(artifactPath);
  const relative = path.relative(receiptRoot, artifactPath).split(path.sep).join("/");
  return {
    role: "component-artifact",
    component,
    ecosystem: "native",
    kind: "native-executable",
    target,
    path: relative,
    sha256: sha256(bytes),
    size_bytes: bytes.length,
  };
}

async function localReceipt({ component, repository, snapshot, target, version, executable }) {
  const receiptRoot = path.join(BUILD_ROOT, "receipts", component);
  const artifactPath = path.join(receiptRoot, "artifacts", path.basename(executable));
  await mkdir(path.dirname(artifactPath), { recursive: true });
  await copyFile(executable, artifactPath);
  if (process.platform !== "win32") {
    await chmod(artifactPath, 0o755);
  }
  const receipt = {
    contract: "ait.release.adapter.receipt/v1",
    repo_name: repository,
    snapshot_id: snapshot,
    version,
    target,
    metadata: {
      package: { version },
      source_build: true,
      publishable: false,
    },
    artifacts: [await artifactRow(component, target, receiptRoot, artifactPath)],
    public_publish: false,
    publishable: false,
  };
  const receiptPath = path.join(receiptRoot, "ait-release.receipt.json");
  await writeFile(receiptPath, `${JSON.stringify(receipt, null, 2)}\n`);
  return receiptPath;
}

function portableRelativePath(value, label, allowDot = false) {
  if (
    typeof value !== "string" ||
    value.length === 0 ||
    value.length > 4096 ||
    value.includes("\\") ||
    value.includes(":") ||
    value.startsWith("/") ||
    /[\0\r\n\t]/u.test(value)
  ) {
    fail(`${label} is not a portable relative path`);
  }
  if (allowDot && value === ".") {
    return value;
  }
  if (value.split("/").some((segment) => segment === "" || segment === "." || segment === "..")) {
    fail(`${label} is not normalized`);
  }
  return value;
}

function identifier(value, label) {
  if (typeof value !== "string" || !/^[A-Za-z0-9][A-Za-z0-9._-]*$/.test(value)) {
    fail(`${label} is not an identifier`);
  }
  return value;
}

function stringArray(value, label, allowEmpty = false) {
  if (
    !Array.isArray(value) ||
    (!allowEmpty && value.length === 0) ||
    value.some((row) => typeof row !== "string")
  ) {
    fail(`${label} must be ${allowEmpty ? "an" : "a non-empty"} string array`);
  }
  if (new Set(value).size !== value.length) {
    fail(`${label} contains duplicates`);
  }
  return value;
}

function commandRows(value, label) {
  if (value === undefined) {
    return [];
  }
  if (
    !Array.isArray(value) ||
    value.length > 16 ||
    value.some(
      (argv) =>
        !Array.isArray(argv) ||
        argv.length === 0 ||
        argv.length > 64 ||
        argv.some(
          (argument) =>
            typeof argument !== "string" ||
            argument.length === 0 ||
            argument.length > 1024 ||
            argument.includes("\0"),
        ),
    )
  ) {
    fail(`${label} contains an invalid direct argv command`);
  }
  return value;
}

async function publicAdapter(repository, version) {
  identifier(repository, "release repository");
  const repositoryRoot = path.join(ROOT, repository);
  await directory(repositoryRoot, `${repository} source subtree`);
  const manifestPath = path.join(repositoryRoot, "ait-release.json");
  await regularFile(manifestPath, `${repository} release adapter`);
  const bytes = await readFile(manifestPath);
  const definition = JSON.parse(bytes.toString("utf8"));
  if (
    definition?.schema !== "ait.release.adapter/v1" ||
    typeof definition?.package !== "object" ||
    definition.package?.version !== version ||
    !Array.isArray(definition?.components) ||
    definition.components.length === 0 ||
    definition.components.length > 64
  ) {
    fail(`${repository} release adapter identity or version is invalid`);
  }
  const licenseFiles = definition.package.license_files ?? [];
  if (!Array.isArray(licenseFiles) || licenseFiles.length === 0 || licenseFiles.length > 8) {
    fail(`${repository} release adapter must declare bounded license material`);
  }
  const licenseKeys = new Set();
  for (const [index, row] of licenseFiles.entries()) {
    const declaredPath = portableRelativePath(row?.path, `license_files[${index}].path`);
    const role = identifier(row?.role, `license_files[${index}].role`);
    if (!new Set(["license", "notice"]).has(role) || licenseKeys.has(`${role}\0${declaredPath}`)) {
      fail(`${repository} release adapter has invalid or duplicate license material`);
    }
    licenseKeys.add(`${role}\0${declaredPath}`);
    await regularFile(path.join(repositoryRoot, declaredPath), `${repository} ${role} material`);
  }
  const componentIds = new Set();
  for (const [index, component] of definition.components.entries()) {
    const componentId = identifier(component?.id, `components[${index}].id`);
    if (componentIds.has(componentId)) {
      fail(`${repository} release adapter contains duplicate component ${componentId}`);
    }
    componentIds.add(componentId);
    identifier(component?.ecosystem, `${componentId} ecosystem`);
    portableRelativePath(component?.working_directory, `${componentId} working directory`, true);
    const dependencyFiles = stringArray(
      component?.dependency_files,
      `${componentId} dependency files`,
    );
    for (const dependency of dependencyFiles) {
      portableRelativePath(dependency, `${componentId} dependency file`);
      await regularFile(path.join(repositoryRoot, dependency), `${componentId} dependency file`);
    }
    const commands = component?.commands;
    if (typeof commands !== "object" || commands === null || Array.isArray(commands)) {
      fail(`${componentId} commands must be an object`);
    }
    for (const phase of ["prepare", "test", "build", "smoke"]) {
      commandRows(commands[phase], `${componentId} ${phase} commands`);
    }
    if (!Array.isArray(component?.artifacts) || component.artifacts.length === 0) {
      fail(`${componentId} artifacts must be non-empty`);
    }
    const artifactKeys = new Set();
    for (const [artifactIndex, artifact] of component.artifacts.entries()) {
      portableRelativePath(artifact?.path, `${componentId} artifact ${artifactIndex} path`);
      const kind = identifier(artifact?.kind, `${componentId} artifact ${artifactIndex} kind`);
      const target = artifact?.target;
      if (target !== undefined) {
        identifier(target, `${componentId} artifact ${artifactIndex} target`);
      }
      const key = `${kind}\0${target ?? ""}`;
      if (artifactKeys.has(key)) {
        fail(`${componentId} contains duplicate artifact key ${key}`);
      }
      artifactKeys.add(key);
    }
  }
  return {
    definition,
    manifestSha256: sha256(bytes),
    repositoryRoot,
  };
}

function selectedPublicComponents(definition, target) {
  return definition.components
    .map((component) => ({
      ...component,
      selectedArtifacts: component.artifacts.filter((artifact) =>
        target === "portable" ? artifact.target === undefined : artifact.target === target,
      ),
    }))
    .filter((component) => component.selectedArtifacts.length > 0);
}

function resolvePublicArgument(argument, values) {
  const replacements = new Map([
    ["$AIT_RELEASE_ID", values.releaseId],
    ["$AIT_RELEASE_VERSION", values.version],
    ["$AIT_RELEASE_COMPONENT", values.component],
    ["$AIT_RELEASE_ECOSYSTEM", values.ecosystem],
    ["$AIT_RELEASE_TARGET", values.target],
    ["$SOURCE_DATE_EPOCH", values.sourceDateEpoch],
  ]);
  return replacements.get(argument) ?? argument;
}

async function receiptArtifact(source, destination, relative, row) {
  await regularFile(source, `built ${row.component ?? row.material_role} artifact`);
  await mkdir(path.dirname(destination), { recursive: true });
  await copyFile(source, destination);
  const bytes = await readFile(destination);
  return {
    ...row,
    path: relative,
    size_bytes: bytes.length,
    sha256: sha256(bytes),
  };
}

async function publicGitComponentReceipt(
  { family, mapping, gitCommit },
  { repository, target, version, outputDirectory },
) {
  if (gitCommit === undefined) {
    fail("public Git component receipts require an exact checked-out Git commit");
  }
  if (!EXPECTED_REPOSITORIES.includes(repository)) {
    fail(`unsupported component repository ${repository}`);
  }
  identifier(target, "component target");
  const mappingRow = mapping.subtrees.find((row) => row.source_repository === repository);
  if (mappingRow === undefined) {
    fail(`source mapping is missing ${repository}`);
  }
  const familyComponents = family.components.filter(
    (component) => component.source_repository === repository,
  );
  if (
    familyComponents.length === 0 ||
    familyComponents.some(
      (component) => component.version !== version || component.source_snapshot !== mappingRow.source_snapshot,
    )
  ) {
    fail(`${repository} version or Snapshot differs from the family manifest`);
  }
  const { definition, manifestSha256, repositoryRoot } = await publicAdapter(repository, version);
  const components = selectedPublicComponents(definition, target);
  exactSet(
    components.map((component) => component.id),
    familyComponents
      .filter((component) =>
        component.artifacts.some((artifact) =>
          target === "portable" ? artifact.targets.length === 0 : artifact.targets.includes(target),
        ),
      )
      .map((component) => component.id),
    `${repository} selected receipt components`,
  );
  if (components.length === 0) {
    fail(`${repository} has no artifacts for ${target}`);
  }
  const outputRoot = path.resolve(outputDirectory);
  const outputRelative = path.relative(ROOT, outputRoot);
  if (
    outputRelative === "" ||
    (!outputRelative.startsWith(`..${path.sep}`) &&
      outputRelative !== ".." &&
      !path.isAbsolute(outputRelative))
  ) {
    fail("component receipt output must be outside the immutable public source tree");
  }
  let outputExists = false;
  try {
    await lstat(outputRoot);
    outputExists = true;
  } catch (error) {
    if (error.code !== "ENOENT") {
      throw error;
    }
  }
  if (outputExists) {
    fail(`component receipt output already exists: ${outputRoot}`);
  }
  const canonicalSourceRoot = await realpath(ROOT);
  const canonicalOutputParent = await realpath(path.dirname(outputRoot)).catch((error) => {
    fail(`component receipt output parent is unavailable: ${error.message}`);
  });
  if (
    canonicalOutputParent === canonicalSourceRoot ||
    canonicalOutputParent.startsWith(`${canonicalSourceRoot}${path.sep}`)
  ) {
    fail("component receipt output parent resolves inside the immutable public source tree");
  }
  await mkdir(outputRoot, { recursive: false });
  const releaseIdentity = [
    PUBLIC_SOURCE_IDENTITY,
    gitCommit,
    repository,
    mappingRow.source_snapshot,
    version,
    target,
    manifestSha256,
  ].join("\0");
  const releaseId = `REL-GIT-${sha256(Buffer.from(releaseIdentity, "utf8"))
    .slice(0, 16)
    .toUpperCase()}`;
  const sourceDateEpoch = mappingRow.source_snapshot_created_at;
  const commandEvidence = [];
  for (const component of components) {
    const workingDirectory = path.join(repositoryRoot, component.working_directory);
    await directory(workingDirectory, `${component.id} working directory`);
    const canonicalRepository = await realpath(repositoryRoot);
    const canonicalWorkingDirectory = await realpath(workingDirectory);
    if (
      canonicalWorkingDirectory !== canonicalRepository &&
      !canonicalWorkingDirectory.startsWith(`${canonicalRepository}${path.sep}`)
    ) {
      fail(`${component.id} working directory escapes its public source subtree`);
    }
    const commandEnv = {
      AIT_RELEASE_ID: releaseId,
      AIT_RELEASE_VERSION: version,
      AIT_RELEASE_COMPONENT: component.id,
      AIT_RELEASE_ECOSYSTEM: component.ecosystem,
      AIT_RELEASE_TARGET: target,
      SOURCE_DATE_EPOCH: sourceDateEpoch,
    };
    for (const phase of ["prepare", "test", "build", "smoke"]) {
      const rows = commandRows(component.commands[phase], `${component.id} ${phase} commands`);
      for (const [commandIndex, argv] of rows.entries()) {
        const resolved = argv.map((argument) =>
          resolvePublicArgument(argument, {
            releaseId,
            version,
            component: component.id,
            ecosystem: component.ecosystem,
            target,
            sourceDateEpoch,
          }),
        );
        run(resolved[0], resolved.slice(1), workingDirectory, commandEnv);
        commandEvidence.push({
          component: component.id,
          phase,
          command_index: commandIndex,
          status: "pass",
        });
      }
    }
  }

  const artifacts = [];
  for (const component of components) {
    for (const artifact of component.selectedArtifacts) {
      const declaredPath = portableRelativePath(
        artifact.path,
        `${component.id} declared artifact path`,
      );
      const relative = `dist/${releaseId}/components/${component.id}/${declaredPath}`;
      const destination = path.join(outputRoot, ...relative.split("/"));
      const source = path.join(repositoryRoot, declaredPath);
      const canonicalRepository = await realpath(repositoryRoot);
      const canonicalSource = await realpath(source);
      if (!canonicalSource.startsWith(`${canonicalRepository}${path.sep}`)) {
        fail(`${component.id} artifact escapes its public source subtree`);
      }
      artifacts.push(
        await receiptArtifact(source, destination, relative, {
          role: "component-artifact",
          component: component.id,
          ecosystem: component.ecosystem,
          kind: artifact.kind,
          target: artifact.target ?? null,
          declared_path: declaredPath,
        }),
      );
    }
  }
  for (const material of definition.package.license_files) {
    const declaredPath = portableRelativePath(material.path, `${repository} legal path`);
    const relative = `dist/${releaseId}/license-material/${material.role}/${declaredPath}`;
    const destination = path.join(outputRoot, ...relative.split("/"));
    const source = path.join(repositoryRoot, declaredPath);
    const canonicalRepository = await realpath(repositoryRoot);
    const canonicalSource = await realpath(source);
    if (!canonicalSource.startsWith(`${canonicalRepository}${path.sep}`)) {
      fail(`${repository} legal material escapes its public source subtree`);
    }
    artifacts.push(
      await receiptArtifact(source, destination, relative, {
        role: "license-material",
        kind: "license-material",
        material_role: material.role,
        declared_path: declaredPath,
        source_path: declaredPath,
      }),
    );
  }

  const releaseManifestRelative = `dist/${releaseId}/ait-release.manifest.json`;
  const releaseManifestPath = path.join(outputRoot, ...releaseManifestRelative.split("/"));
  await mkdir(path.dirname(releaseManifestPath), { recursive: true });
  const releaseManifest = {
    contract: "ait.release.public-git.manifest/v1",
    builder: "ait_public_git_adapter_v1",
    release_id: releaseId,
    repo_name: repository,
    version,
    target,
    git_commit: gitCommit,
    source_snapshot: mappingRow.source_snapshot,
    source_mapping_sha256: sha256(await readFile(SOURCE_MAPPING)),
    adapter_manifest_sha256: manifestSha256,
    components: components.map((component) => ({
      component: component.id,
      ecosystem: component.ecosystem,
      status: "pass",
      artifact_count: component.selectedArtifacts.length,
    })),
    artifacts: artifacts.filter((row) => row.role === "component-artifact"),
    license_material: artifacts.filter((row) => row.role === "license-material"),
    source_date_epoch: sourceDateEpoch,
  };
  await writeFile(releaseManifestPath, `${JSON.stringify(releaseManifest, null, 2)}\n`);
  const releaseManifestBytes = await readFile(releaseManifestPath);
  artifacts.push({
    role: "release-manifest",
    kind: "manifest",
    path: releaseManifestRelative,
    size_bytes: releaseManifestBytes.length,
    sha256: sha256(releaseManifestBytes),
  });
  const checksumRelative = `dist/${releaseId}/ait-release.sha256`;
  const checksumPath = path.join(outputRoot, ...checksumRelative.split("/"));
  const checksumText = `${artifacts.map((row) => `${row.sha256}  ${row.path}`).join("\n")}\n`;
  await writeFile(checksumPath, checksumText);
  const checksumBytes = await readFile(checksumPath);
  artifacts.push({
    role: "release-checksum",
    kind: "checksum",
    path: checksumRelative,
    size_bytes: checksumBytes.length,
    sha256: sha256(checksumBytes),
  });
  artifacts.sort((left, right) =>
    `${left.role}\0${left.component ?? ""}\0${left.path}`.localeCompare(
      `${right.role}\0${right.component ?? ""}\0${right.path}`,
      "en",
    ),
  );

  const sourceMappingSha256 = sha256(await readFile(SOURCE_MAPPING));
  const receipt = {
    contract: "ait.release.public-git.receipt/v1",
    command: "release public-git adapter build",
    release_id: releaseId,
    repo_name: repository,
    version,
    line: "main",
    line_name: "main",
    snapshot_id: mappingRow.source_snapshot,
    manifest_hash: mappingRow.source_manifest_hash,
    profile: "generic-command",
    package_name: definition.package.name,
    package_version: definition.package.version,
    package_requires_python: null,
    package: {
      name: definition.package.name,
      version: definition.package.version,
      description: definition.package.description ?? null,
      requires_python: null,
      license_files: definition.package.license_files,
      adapter_contract: "ait.release.adapter/v1",
    },
    status: "built",
    checks: [
      { code: "public_source", status: "pass", blocking: false },
      { code: "dependency_authority", status: "pass", blocking: false },
      { code: "component_commands", status: "pass", blocking: false },
      { code: "artifact_integrity", status: "pass", blocking: false },
    ],
    check_summary: { total: 4, passed: 4, failed: 0, blocking: 0, decision: "pass" },
    artifacts,
    formula: {},
    metadata: {
      package: {
        name: definition.package.name,
        version: definition.package.version,
        description: definition.package.description ?? null,
        requires_python: null,
        license_files: definition.package.license_files,
        adapter_contract: "ait.release.adapter/v1",
      },
      profile: "generic-command",
      profile_settings: {
        builder: "ait_public_git_adapter_v1",
        argument_tokens: [
          "$AIT_RELEASE_ID",
          "$AIT_RELEASE_VERSION",
          "$AIT_RELEASE_COMPONENT",
          "$AIT_RELEASE_ECOSYSTEM",
          "$AIT_RELEASE_TARGET",
          "$SOURCE_DATE_EPOCH",
        ],
      },
      source_snapshot_created_at: sourceDateEpoch,
      release_adapter: {
        contract: "ait.release.adapter/v1",
        manifest_path: "ait-release.json",
        manifest_sha256: manifestSha256,
        component_count: definition.components.length,
        declared_artifact_count: definition.components.reduce(
          (count, component) => count + component.artifacts.length,
          0,
        ),
        license_material_count: definition.package.license_files.length,
        definition,
      },
      build: {
        builder: "ait_public_git_adapter_v1",
        adapter_contract: "ait.release.adapter/v1",
        adapter_manifest_sha256: manifestSha256,
        dist_dir: `dist/${releaseId}`,
        manifest_path: releaseManifestRelative,
        checksum_path: checksumRelative,
        built_at: sourceDateEpoch,
        source_date_epoch: sourceDateEpoch,
        component_count: components.length,
        declared_artifact_count: components.reduce(
          (count, component) => count + component.selectedArtifacts.length,
          0,
        ),
        license_material_count: definition.package.license_files.length,
        components: components.map((component) => ({
          component: component.id,
          ecosystem: component.ecosystem,
          status: "pass",
          command_count: commandEvidence.filter((row) => row.component === component.id).length,
          artifact_count: component.selectedArtifacts.length,
        })),
        command_evidence: commandEvidence,
        command_execution: "direct_argv_without_implicit_shell",
        registry_publish: false,
      },
    },
    authority: {
      source: "public_git_commit",
      public_source_identity: PUBLIC_SOURCE_IDENTITY,
      git_commit: gitCommit,
      coordinator_snapshot: mapping.coordinator_snapshot,
      source_snapshot: mappingRow.source_snapshot,
      source_manifest_hash: mappingRow.source_manifest_hash,
      source_mapping_path: "ait-monorepo-source.json",
      source_mapping_sha256: sourceMappingSha256,
      source_content_sha256: mapping.content_sha256,
      subtree_path: mappingRow.path,
      subtree_exported_content_sha256: mappingRow.exported_content_sha256,
      persistence: "ci_artifact_bundle",
      local_release_authority: "not_activated",
      remote_publish_supported: false,
    },
    artifact_selection: target === "portable" ? "portable" : undefined,
    target: target === "portable" ? undefined : target,
    public_publish: false,
    publishable: false,
    created_at: sourceDateEpoch,
    updated_at: sourceDateEpoch,
  };
  for (const key of ["artifact_selection", "target"]) {
    if (receipt[key] === undefined) {
      delete receipt[key];
    }
  }
  const receiptPath = path.join(outputRoot, "ait-release.receipt.json");
  await writeFile(receiptPath, `${JSON.stringify(receipt, null, 2)}\n`);
  const receiptBytes = await readFile(receiptPath);
  const componentArtifactCount = artifacts.filter((row) => row.role === "component-artifact").length;
  const ciEvidence = {
    contract: "ait.release.component-ci-evidence/v2",
    status: "pass",
    repo_name: repository,
    source_snapshot: mappingRow.source_snapshot,
    version,
    target,
    runner: {
      label: process.env.AIT_RELEASE_RUNNER_LABEL ?? "unknown",
      os: process.env.AIT_RELEASE_RUNNER_OS ?? "unknown",
      architecture: process.env.AIT_RELEASE_RUNNER_ARCH ?? "unknown",
      image: process.env.AIT_RELEASE_RUNNER_IMAGE ?? "unknown",
    },
    platform_floor: {
      kind: process.env.AIT_RELEASE_PLATFORM_FLOOR_KIND ?? "unknown",
      value: process.env.AIT_RELEASE_PLATFORM_FLOOR ?? "unknown",
    },
    git_commit: gitCommit,
    source_mapping_sha256: sourceMappingSha256,
    receipt_sha256: sha256(receiptBytes),
    component_artifact_count: componentArtifactCount,
    recorded_artifact_count: artifacts.length,
    source_authority: "public_git_commit",
    registry_publish: false,
    public_publish: false,
  };
  await writeFile(
    path.join(outputRoot, "ci-run.evidence.json"),
    `${JSON.stringify(ciEvidence, null, 2)}\n`,
  );
  process.stdout.write(`${JSON.stringify(receipt, null, 2)}\n`);
}

async function inventory(root) {
  const files = [];
  async function visit(current, relative = "") {
    const rows = await readdir(current, { withFileTypes: true });
    rows.sort((left, right) => left.name.localeCompare(right.name, "en"));
    for (const row of rows) {
      const childRelative = relative === "" ? row.name : `${relative}/${row.name}`;
      const child = path.join(current, row.name);
      if (row.isDirectory()) {
        await visit(child, childRelative);
      } else if (row.isFile()) {
        const bytes = await readFile(child);
        const entry = await stat(child);
        files.push({ path: childRelative, sha256: sha256(bytes), size_bytes: entry.size });
      } else {
        fail(`build output contains a symlink or special file: ${childRelative}`);
      }
    }
  }
  await visit(root);
  return files;
}

async function build({ family, mapping }, skipTests) {
  const target = hostTarget();
  const suffix = process.platform === "win32" ? ".exe" : "";
  await rm(BUILD_ROOT, { recursive: true, force: true });
  await rm(OUTPUT_ROOT, { recursive: true, force: true });
  await mkdir(OUTPUT_ROOT, { recursive: true });

  const coreTarget = path.join(BUILD_ROOT, "cargo", "core");
  const serverTarget = path.join(BUILD_ROOT, "cargo", "server");
  const runnerTarget = path.join(BUILD_ROOT, "cargo", "runner");
  run("cargo", [
    "build", "--locked", "--release",
    "--manifest-path", path.join(ROOT, "ait-core", "rust", "Cargo.toml"),
    "--target-dir", coreTarget,
    "-p", "ait-cli", "--bin", "ait-cli", "--bin", "ait-agent",
  ]);
  run("cargo", [
    "build", "--locked", "--release",
    "--manifest-path", path.join(ROOT, "ait-server", "rust", "Cargo.toml"),
    "--target-dir", serverTarget,
    "-p", "ait-server", "--bin", "ait-server",
  ]);
  run("cargo", [
    "build", "--locked", "--release",
    "--manifest-path", path.join(ROOT, "ait-runner", "Cargo.toml"),
    "--target-dir", runnerTarget,
    "--bin", "ait-runner",
  ]);

  const binOutput = path.join(OUTPUT_ROOT, target, "bin");
  const built = {
    ait: path.join(binOutput, executableName("ait")),
    "ait-agent": path.join(binOutput, executableName("ait-agent")),
    "ait-server": path.join(binOutput, executableName("ait-server")),
    "ait-runner": path.join(binOutput, executableName("ait-runner")),
  };
  await copyExecutable(path.join(coreTarget, "release", `ait-cli${suffix}`), built.ait);
  await copyExecutable(path.join(coreTarget, "release", `ait-agent${suffix}`), built["ait-agent"]);
  await copyExecutable(path.join(serverTarget, "release", `ait-server${suffix}`), built["ait-server"]);
  await copyExecutable(path.join(runnerTarget, "release", `ait-runner${suffix}`), built["ait-runner"]);

  const python = process.env.PYTHON || (process.platform === "win32" ? "python" : "python3");
  const pythonOutput = path.join(OUTPUT_ROOT, target, "python");
  await mkdir(pythonOutput, { recursive: true });
  run(python, [
    "-m", "pip", "wheel", "--disable-pip-version-check", "--no-deps",
    "--wheel-dir", pythonOutput, path.join(ROOT, "ait-python"),
  ]);

  if (!skipTests) {
    run("npm", ["test", "--ignore-scripts"], path.join(ROOT, "ait-node"));
  }
  const npmOutput = path.join(OUTPUT_ROOT, target, "npm");
  await mkdir(npmOutput, { recursive: true });
  run("npm", [
    "pack", "--ignore-scripts", "--pack-destination", npmOutput,
    path.join(ROOT, "ait-node"),
  ]);

  const snapshots = new Map(
    mapping.subtrees.map((row) => [row.source_repository, row.source_snapshot]),
  );
  const coreReceipt = await localReceipt({
    component: "ait",
    repository: "ait-core",
    snapshot: snapshots.get("ait-core"),
    target,
    version: family.family.version,
    executable: built.ait,
  });
  const serverReceipt = await localReceipt({
    component: "ait-server",
    repository: "ait-server",
    snapshot: snapshots.get("ait-server"),
    target,
    version: family.family.version,
    executable: built["ait-server"],
  });
  const payloadTool = path.join(ROOT, "ait-node", "release", "npm-payload-package.mjs");
  for (const [component, repository, receipt] of [
    ["ait", "ait-core", coreReceipt],
    ["ait-server", "ait-server", serverReceipt],
  ]) {
    run("node", [
      payloadTool, "build",
      "--component", component,
      "--target", target,
      "--version", family.family.version,
      "--receipt", receipt,
      "--license", path.join(ROOT, repository, "LICENSE"),
      "--notice", path.join(ROOT, repository, "NOTICE"),
      "--out-dir", npmOutput,
    ]);
  }

  const files = await inventory(OUTPUT_ROOT);
  const manifest = {
    contract: "ait.release.local-source-build/v1",
    family_version: family.family.version,
    family_tag: family.family.tag,
    coordinator_snapshot: mapping.coordinator_snapshot,
    target,
    source_mapping_sha256: sha256(await readFile(SOURCE_MAPPING)),
    source_content_sha256: mapping.content_sha256,
    artifacts: files,
    receipts: {
      authority: "local-source-build-only",
      publishable: false,
    },
    public_publish: false,
    publishable: false,
  };
  await writeFile(
    path.join(OUTPUT_ROOT, "ait-local-source-build.json"),
    `${JSON.stringify(manifest, null, 2)}\n`,
  );
  process.stdout.write(`${JSON.stringify(manifest, null, 2)}\n`);
}

async function main() {
  const args = process.argv.slice(2);
  const flags = new Set(["--validate-only", "--skip-tests", "--component-receipt"]);
  const valueOptions = new Set([
    "--repository",
    "--target",
    "--version",
    "--git-commit",
    "--out-dir",
  ]);
  const selectedFlags = new Set();
  const values = new Map();
  for (let index = 0; index < args.length; index += 1) {
    const argument = args[index];
    if (flags.has(argument)) {
      if (selectedFlags.has(argument)) {
        fail(`duplicate option ${argument}`);
      }
      selectedFlags.add(argument);
      continue;
    }
    if (valueOptions.has(argument)) {
      const value = args[index + 1];
      if (value === undefined || value.startsWith("--") || values.has(argument)) {
        fail(`option ${argument} requires one value and may appear only once`);
      }
      values.set(argument, value);
      index += 1;
      continue;
    }
    fail(`unsupported argument ${argument}`);
  }
  const gitCommit = values.get("--git-commit");
  const validated = await validateBuildInputs(gitCommit);
  if (selectedFlags.has("--component-receipt")) {
    if (selectedFlags.has("--validate-only") || selectedFlags.has("--skip-tests")) {
      fail("--component-receipt cannot be combined with validation-only or local-build flags");
    }
    for (const required of ["--repository", "--target", "--version", "--git-commit", "--out-dir"]) {
      if (!values.has(required)) {
        fail(`--component-receipt requires ${required}`);
      }
    }
    await publicGitComponentReceipt(validated, {
      repository: values.get("--repository"),
      target: values.get("--target"),
      version: values.get("--version"),
      outputDirectory: values.get("--out-dir"),
    });
    return;
  }
  if (
    [...values.keys()].some((option) => option !== "--git-commit") ||
    (gitCommit !== undefined && !selectedFlags.has("--validate-only"))
  ) {
    fail("source identity options apply only to --validate-only or --component-receipt");
  }
  if (selectedFlags.has("--validate-only")) {
    if (selectedFlags.has("--skip-tests")) {
      fail("--validate-only and --skip-tests are mutually exclusive");
    }
    process.stdout.write("ait-native public source build contract: pass\n");
    return;
  }
  await build(validated, selectedFlags.has("--skip-tests"));
}

main().catch((error) => {
  process.stderr.write(`ait-native source build failed: ${error.message}\n`);
  process.exitCode = 1;
});

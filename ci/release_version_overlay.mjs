#!/usr/bin/env node

import path from "node:path";
import { existsSync, lstatSync, realpathSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { replaceComponentSources } from "./release_stable_prepare.mjs";

const COMPONENTS = new Map([
  ["ait-core", "core"],
  ["ait-server", "server"],
  ["ait-runner", "runner"],
  ["ait-python", "python"],
  ["ait-node", "node"],
]);
const SNAPSHOT_RE = /^SNP-[0-9A-F]{12}$/;
const VERSION_RE = /^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)(-rc\.[1-9][0-9]*)?$/;

function fail(message, code = 65) {
  process.stderr.write(`${message}\n`);
  process.exit(code);
}

function usage() {
  fail(
    "usage: release_version_overlay.mjs --root <absolute-source-root> --repository <ait-core|ait-server|ait-runner|ait-python|ait-node> --from <version> --to <version> [--core-snapshot <SNP>]",
    64,
  );
}

function main() {
  const argv = process.argv.slice(2);
  const options = {};
  while (argv.length > 0) {
    const key = argv.shift();
    const value = argv.shift();
    if (!key?.startsWith("--") || value === undefined || options[key] !== undefined) usage();
    options[key] = value;
  }
  const root = options["--root"];
  const repository = options["--repository"];
  const from = options["--from"];
  const to = options["--to"];
  const coreSnapshot = options["--core-snapshot"] ?? null;
  if (typeof root !== "string" || !path.isAbsolute(root) || !COMPONENTS.has(repository) ||
      !VERSION_RE.test(from ?? "") || !VERSION_RE.test(to ?? "")) usage();
  if (!existsSync(root) || !lstatSync(root).isDirectory() || lstatSync(root).isSymbolicLink()) {
    fail(`version overlay root must be a real directory: ${root}`, 66);
  }
  if (from === to) fail("version overlay requires two different versions");
  const component = COMPONENTS.get(repository);
  if (["runner", "python", "node"].includes(component) && !SNAPSHOT_RE.test(coreSnapshot ?? "")) {
    fail(`version overlay for ${repository} requires the bound Core Snapshot`);
  }
  let changed;
  try {
    changed = replaceComponentSources(path.resolve(root), component, from, to, coreSnapshot);
  } catch (error) {
    fail(`version overlay failed for ${repository}: ${error.message}`);
  }
  process.stdout.write(`${JSON.stringify({
    contract: "ait.release.version-overlay/v1",
    repository,
    from,
    to,
    changed_paths: [...changed].sort(),
  })}\n`);
}

if (process.argv[1] && realpathSync(path.resolve(process.argv[1])) === realpathSync(fileURLToPath(import.meta.url))) {
  main();
}

#!/usr/bin/env node

import { createHash } from "node:crypto";
import { existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import path from "node:path";

const PHASES = [
  "source_preflight",
  "prior_qualification",
  "component_release",
  "candidate_freeze",
  "public_qualification",
  "candidate_admission",
  "component_receipts",
  "clean_host_qualification",
  "web_admission",
  "tag",
  "protected_promotion",
  "endpoint_publication",
  "winget_submission",
  "latest_alias",
  "closeout",
];

function fail(message, code = 65) {
  process.stderr.write(`${message}\n`);
  process.exit(code);
}

function usage() {
  fail("usage: release_conductor_init.mjs --spec <absolute-json> --recipe <absolute-json> --plan <absolute-json>", 64);
}

function parseCli(argv) {
  const options = {};
  while (argv.length > 0) {
    const key = argv.shift();
    const value = argv.shift();
    if (!key?.startsWith("--") || value === undefined || options[key] !== undefined) usage();
    options[key] = value;
  }
  if (!options["--spec"] || !options["--recipe"] || !options["--plan"]) usage();
  return options;
}

function absolute(value, label) {
  if (typeof value !== "string" || !path.isAbsolute(value)) fail(`${label} must be an absolute path`);
  return path.resolve(value);
}

function readJson(file, label) {
  try {
    return JSON.parse(readFileSync(file, "utf8"));
  } catch {
    fail(`${label} is not valid JSON`);
  }
}

function sha256(file) {
  return createHash("sha256").update(readFileSync(file)).digest("hex");
}

function stableAdvance(prior, next) {
  const before = /^(\d+)\.(\d+)\.(\d+)$/.exec(prior ?? "");
  const after = /^(\d+)\.(\d+)\.(\d+)$/.exec(next ?? "");
  if (!before || !after) return false;
  const old = before.slice(1).map(Number);
  const current = after.slice(1).map(Number);
  return (
    (current[0] === old[0] && current[1] === old[1] && current[2] === old[2] + 1) ||
    (current[0] === old[0] && current[1] === old[1] + 1 && current[2] === 0)
  );
}

function lookup(context, selector) {
  if (!/^[a-zA-Z0-9_-]+(?:\.[a-zA-Z0-9_-]+)*$/.test(selector)) fail(`template selector is invalid: ${selector}`);
  let value = context;
  for (const part of selector.split(".")) {
    if (value === null || typeof value !== "object" || !Object.hasOwn(value, part)) {
      fail(`template selector is missing: ${selector}`);
    }
    value = value[part];
  }
  return value;
}

function expand(value, context) {
  if (Array.isArray(value)) return value.map((row) => expand(row, context));
  if (value !== null && typeof value === "object") {
    return Object.fromEntries(Object.entries(value).map(([key, row]) => [key, expand(row, context)]));
  }
  if (typeof value !== "string") return value;
  const exact = /^\{\{([^{}]+)\}\}$/.exec(value);
  if (exact) return structuredClone(lookup(context, exact[1]));
  return value.replace(/\{\{([^{}]+)\}\}/g, (_, selector) => {
    const replacement = lookup(context, selector);
    if (!["string", "number", "boolean"].includes(typeof replacement)) {
      fail(`embedded template selector is not scalar: ${selector}`);
    }
    return String(replacement);
  });
}

function compileActions(actions, context, phase) {
  const compiled = [];
  for (const action of actions) {
    if (action !== null && typeof action === "object" && Object.hasOwn(action, "$for_each")) {
      const keys = Object.keys(action).sort();
      if (JSON.stringify(keys) !== JSON.stringify(["$for_each", "template"])) {
        fail(`${phase} expansion contains unsupported fields`);
      }
      const rows = lookup(context, action.$for_each);
      if (!Array.isArray(rows) || rows.length === 0) fail(`${phase} expansion source must be a non-empty array`);
      rows.forEach((item, index) => compiled.push(expand(action.template, { ...context, item, item_index: index })));
    } else {
      compiled.push(expand(action, context));
    }
  }
  return compiled;
}

const cli = parseCli(process.argv.slice(2));
const specPath = absolute(cli["--spec"], "spec");
const recipePath = absolute(cli["--recipe"], "recipe");
const planPath = absolute(cli["--plan"], "plan");
if (existsSync(planPath)) fail("release conductor plan output already exists", 73);

const spec = readJson(specPath, "release conductor spec");
const recipe = readJson(recipePath, "release conductor recipe");
if (spec.contract !== "ait.release.conductor-spec/v1") fail("release conductor spec contract is invalid");
if (recipe.contract !== "ait.release.conductor-recipe/v1") fail("release conductor recipe contract is invalid");
if (spec.release?.channel !== "stable" || !stableAdvance(spec.release.prior_version, spec.release.version)) {
  fail("release conductor spec requires one exact stable patch or minor advance");
}
const recordsRoot = absolute(spec.records_root, "records_root");
if (!planPath.startsWith(`${recordsRoot}${path.sep}`)) fail("generated plan must remain below records_root");
if (!Array.isArray(recipe.phases) || JSON.stringify(recipe.phases.map((row) => row.id)) !== JSON.stringify(PHASES)) {
  fail("release conductor recipe phase inventory is incomplete or out of order");
}
if (spec.values !== undefined && (spec.values === null || typeof spec.values !== "object" || Array.isArray(spec.values))) {
  fail("release conductor values must be an object");
}
if (spec.collections !== undefined && (spec.collections === null || typeof spec.collections !== "object" || Array.isArray(spec.collections))) {
  fail("release conductor collections must be an object");
}
const context = {
  release: { ...spec.release, tag: `v${spec.release.version}` },
  records_root: recordsRoot,
  spec_path: specPath,
  recipe_path: recipePath,
  values: spec.values ?? {},
  collections: spec.collections ?? {},
};
const phases = recipe.phases.map((phase) => ({
  id: phase.id,
  actions: compileActions(phase.actions ?? [], context, phase.id),
}));
if (phases.some((phase) => phase.actions.length === 0)) fail("generated release plan contains an empty phase");
const serialized = JSON.stringify({
  contract: "ait.release.conductor-plan/v1",
  source: {
    spec_sha256: sha256(specPath),
    recipe_sha256: sha256(recipePath),
  },
  release: context.release,
  records_root: recordsRoot,
  phases,
}, null, 2);
if (/\{\{[^{}]+\}\}/.test(serialized)) fail("generated release plan contains an unresolved selector");
if (/[A-Z]+T-[0-9]{4}\/C-[0-9]{2}/.test(serialized)) fail("generated release plan exposes an internal Change reference");
mkdirSync(path.dirname(planPath), { recursive: true, mode: 0o700 });
writeFileSync(planPath, `${serialized}\n`, { mode: 0o600, flag: "wx" });
process.stdout.write(`${planPath}\n`);

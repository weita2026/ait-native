#!/usr/bin/env node

import { createHash } from "node:crypto";
import { existsSync, mkdirSync, readFileSync, readdirSync, lstatSync, writeFileSync } from "node:fs";
import path from "node:path";
import { inspectPreparedOperation, readAdmissionJson, qualifiedOperationInputs, qualifiedConductorPhases } from "./release_stable_prepare.mjs";
import { compositionInputFiles } from "./release_web_composition_inputs.mjs";
import { validateComposedWebInputs } from "./release_web_controls.mjs";
import { validatePriorComposition } from "./release_web_composition.mjs";
import { boundedWebPhases, validateCompositionSelection } from "./release_web_bounded.mjs";
import { priorWebInput } from "./release_web_recovery_identity.mjs";

const PHASES = [
  "source_preflight",
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

function controlFiles(root) {
  const files = {};
  if (!root) return files;
  const walk = directory => {
    for (const name of readdirSync(directory).sort()) {
      const file = path.join(directory, name), stat = lstatSync(file);
      if (stat.isSymbolicLink()) fail(`release control symlink is not allowed: ${file}`);
      if (stat.isDirectory()) walk(file);
      else if (stat.isFile()) files[file] = sha256(file);
    }
  };
  for (const relative of ["ci", "release", ".github/workflows"]) if (existsSync(path.join(root, relative))) walk(path.join(root, relative));
  for (const relative of ["ait-external.toml", "ait-external.lock", "ait-release.json", "ait-release-family.json", "docs/distribution.md"]) {
    const file = path.join(root, relative);
    if (existsSync(file)) files[file] = sha256(file);
  }
  return files;
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
let phases = recipe.phases.map((phase) => ({
  id: phase.id,
  actions: compileActions(phase.actions ?? [], context, phase.id),
}));
if (phases.some((phase) => phase.actions.length === 0)) fail("generated release plan contains an empty phase");
const controls = controlFiles(spec.values?.native_root);
const closeouts = spec.collections?.component_closeouts;
if (Array.isArray(closeouts) && closeouts.length === 5 && closeouts.every(row => row.finish_local_before_ready === false)) {
  const preparationPath = path.join(recordsRoot, "preparation-state.json");
  const acceptedPath = absolute(spec.values?.accepted_input_path, "accepted_input_path");
  try {
    if (sha256(acceptedPath) !== spec.values.accepted_input_sha256) throw new Error("accepted input differs from compiled spec");
    const ait = process.env.AIT_RELEASE_AIT_BIN ?? path.join(spec.values.core_root, ".ait/cargo-target/release/ait-cli");
    inspectPreparedOperation(preparationPath, acceptedPath, (cwd, argv, missing) => readAdmissionJson(ait, cwd, argv, missing));
    for (const file of [preparationPath, acceptedPath, spec.values.web_components]) controls[file] = sha256(file);
    const accepted = readJson(acceptedPath, "accepted release input"), qualified = qualifiedOperationInputs(accepted, true);
    if (qualified) {
      if (["conductor-state.json", "conductor-checkpoints", "conductor-receipts", "conductor-transactions"].some(name => existsSync(path.join(recordsRoot, name)))) {
        throw new Error("fresh qualified binding rejects existing conductor state or checkpoints");
      }
      Object.assign(controls, qualified.files);
      phases = qualifiedConductorPhases(phases, accepted, qualified);
    }
  } catch (error) { fail(error.message, 75); }
}
try { phases = boundedWebPhases(phases, spec); } catch (error) { fail(error.message, 75); }
const composition = [];
for (const action of phases.flatMap(phase => phase.actions)) {
  if (action.type !== "command" || !action.argv?.includes(path.join(spec.values?.native_root ?? "", "ci/release_web_admission.sh"))) continue;
  const argument = flag => {
    if (action.argv.filter(value => value === flag).length !== 1) fail(`admission invocation has ambiguous ${flag}`);
    return absolute(action.argv[action.argv.indexOf(flag) + 1], flag);
  };
  const workingInputPath = argument("--input"), outputRoot = argument("--output-dir");
  const inside = path.dirname(workingInputPath) === outputRoot || workingInputPath.startsWith(outputRoot + path.sep);
  const bounded = phases.find(phase => phase.id === "web_admission")?.actions.some(row => row.id === "bounded-web-1");
  if (inside && !bounded) continue;
  const inputPath = inside ? priorWebInput(spec.values.native_root, spec.release)?.inputPath : workingInputPath;
  if (!inputPath) fail("bounded WT correction has no reviewed prior input");
  if (outputRoot !== path.join(recordsRoot, "web-admission")) fail("composition output must use the replacement web-admission area");
  composition.push({ inputPath, outputRoot, repository: argument("--repository") });
}
if (composition.length > 1) fail("replacement composition invocation is ambiguous");
try {
  if (composition.length) Object.assign(controls, compositionInputFiles(composition[0].inputPath, absolute(spec.values.native_root, "native_root"), spec));
} catch (error) { fail(error.message, 75); }
const plan = {
  contract: "ait.release.conductor-plan/v1",
  source: {
    spec_sha256: sha256(specPath),
    recipe_sha256: sha256(recipePath),
    control_files: controls,
  },
  release: context.release,
  records_root: recordsRoot,
  phases,
};
const serialized = JSON.stringify(plan, null, 2);
if (/\{\{[^{}]+\}\}/.test(serialized)) fail("generated release plan contains an unresolved selector");
if (/[A-Z]+T-[0-9]{4}\/C-[0-9]{2}/.test(serialized)) fail("generated release plan exposes an internal Change reference");
try {
  if (composition.length) {
    const { inputPath, outputRoot, repository } = composition[0];
    const { context, matrix } = validateComposedWebInputs(inputPath, readJson(inputPath, "composition input"), repository, outputRoot,
      { root: recordsRoot, file: planPath, bytes: Buffer.from(serialized + "\n"), plan });
    validatePriorComposition(context, matrix);
    validateCompositionSelection(spec, inputPath);
    for (const file of Object.keys(controls)) context.pinned(file);
  }
} catch (error) { fail(error.message, 75); }
mkdirSync(path.dirname(planPath), { recursive: true, mode: 0o700 });
writeFileSync(planPath, `${serialized}\n`, { mode: 0o600, flag: "wx" });
process.stdout.write(`${planPath}\n`);

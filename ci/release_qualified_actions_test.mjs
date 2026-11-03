import assert from "node:assert/strict";
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import { qualifiedConductorPhases } from "./release_stable_prepare.mjs";
import { correctedCohort, qualificationCacheActions, qualifiedApplicabilityCaches, validateQualifiedCleanHostActions } from "./release_web_applicability.mjs";

// Real producer functions feed the validator directly. This inventory fixture
// executes no release phase and creates no qualification result.
const root = mkdtempSync(path.join(tmpdir(), "ait-qualified-actions-"));
try {
  const native = path.join(root, "native"), publicSource = path.join(root, "public");
  const accepted = { records_root: root, github_repository: "owner/repository", paths: {
    native_root: native, public_source: publicSource, web_components: path.join(root, "web-components.json") } };
  writeFileSync(accepted.paths.web_components, JSON.stringify(correctedCohort));
  const recipe = JSON.parse(readFileSync(new URL("../release/stable-conductor.recipe.json", import.meta.url)));
  const substitutions = { "{{records_root}}": root, "{{release.version}}": "1.2.0", "{{values.native_root}}": native,
    "{{values.public_source}}": publicSource, "{{values.github_repository}}": accepted.github_repository };
  const compile = value => typeof value === "string" ? Object.entries(substitutions).reduce((v, [from, to]) => v.replaceAll(from, to), value) :
    Array.isArray(value) ? value.map(compile) : value && typeof value === "object" ? Object.fromEntries(Object.entries(value).map(([key, item]) => [key, compile(item)])) : value;
  const phase = compile(recipe.phases.find(row => row.id === "clean_host_qualification"));
  const qualified = { root: path.join(root, "prior"), qualificationRoot: path.join(root, "q-source"),
    qualification: { workflow: { run_id: 101 } }, receipts: { source_dossier: { workflow_run_id: 102 } },
    candidate: { qualification: { workflow_run_id: 103 } } };
  const three = qualifiedConductorPhases([phase], accepted, qualified)[0].actions;
  assert.equal(three[0].type, "command"); assert.equal(three[0].mutation, false);
  assert.equal(three[0].argv[three[0].argv.indexOf("--run-id") + 1], "103");
  const validate = (actions, caches) => validateQualifiedCleanHostActions(actions, native, root, "1.2.0", caches);
  const direct = [{ id: "web_admission", actions: [{ id: "run-web-admission" }] }];
  assert.deepEqual(qualifiedApplicabilityCaches(accepted, direct), []);
  validate(three, []);
  const bounded = [{ id: "web_admission", actions: Array.from({ length: 5 }, (_, index) => ({ id: `bounded-web-${index + 1}` })) }];
  const caches = qualifiedApplicabilityCaches(accepted, bounded);
  assert.deepEqual(caches, qualificationCacheActions(native, root, accepted.github_repository));
  assert.equal(caches.length, 2);
  const five = [...three, ...caches]; validate(five, caches);
  assert.throws(() => validate(three, caches), /inventory/);
  assert.throws(() => validate(five, []), /inventory/);
  writeFileSync(accepted.paths.web_components, JSON.stringify({ ...correctedCohort, core: "SNP-AAAAAAAAAAAA" }));
  assert.deepEqual(qualifiedApplicabilityCaches(accepted, bounded), []);
  writeFileSync(accepted.paths.web_components, JSON.stringify(correctedCohort));

  for (const [name, mutate] of [
    ["missing qualification", actions => actions.shift()],
    ["missing distribution cache", actions => actions.splice(1, 1)],
    ["missing host", actions => actions.splice(2, 1)],
    ["missing one applicability cache", actions => actions.pop()],
    ["missing both applicability caches", actions => actions.splice(3)],
    ["unknown action", actions => actions.push({ ...actions[4], id: "unexpected-cache" })],
    ["duplicate cache", actions => actions.push(structuredClone(actions[4]))],
    ["reordered prerequisites", actions => [actions[1], actions[2]] = [actions[2], actions[1]]],
    ["reordered applicability caches", actions => [actions[3], actions[4]] = [actions[4], actions[3]]],
    ["host mutation", actions => { actions[2].mutation = false; }],
    ["host command", actions => { actions[2].argv[1] = "publish"; }],
    ["host probe", actions => { actions[2].probe.argv[3] = path.join(root, "foreign-cache"); }],
    ["host output", actions => { actions[2].outputs = [path.join(root, "foreign-bin")]; }],
    ["cache mutation", actions => { actions[3].mutation = true; }],
    ["cache type", actions => { actions[3].type = "github_workflow"; }],
    ["cache cwd", actions => { actions[3].cwd = publicSource; }],
    ["cache executable", actions => { actions[3].argv[0] = "/foreign/node"; }],
    ["cache program", actions => { actions[3].argv[3] += " process.exit(0);"; }],
    ["cache module", actions => { actions[3].argv[4] = path.join(native, "ci/foreign.mjs"); }],
    ["cache kind", actions => { actions[3].argv[5] = "candidate"; }],
    ["cache records root", actions => { actions[3].argv[6] = path.join(root, "foreign-operation"); }],
    ["cache provider repository", actions => { actions[3].argv[7] = "foreign/repository"; }],
    ["cache output argument", actions => { actions[3].argv[8] = path.join(root, "wrong-cache"); }],
    ["cache declared output", actions => { actions[3].outputs = [path.join(root, "wrong-cache")]; }],
    ["extra cache bindings", actions => { actions[3].bindings = { run_id: { path: path.join(root, "candidate.json"), format: "json", selector: "other.id" } }; }],
    ["cache adoption", actions => { actions[3].adopt = { cwd: native, argv: ["true"] }; }],
  ]) {
    const actions = structuredClone(five); mutate(actions);
    assert.throws(() => validate(actions, caches), undefined, name);
  }
  // The unchanged three-action route keeps its original strict host guards.
  for (const index of [0, 1, 2]) {
    const actions = structuredClone(three); actions.splice(index, 1); assert.throws(() => validate(actions, []));
  }
  const unguarded = structuredClone(three); delete unguarded[2].probe;
  assert.throws(() => validate(unguarded, []), /guarded host/);
} finally { rmSync(root, { recursive: true, force: true }); }
console.log("Qualified normal producers validate exact three/five inventories, cache bindings and host guards");

#!/usr/bin/env node
import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { readFileSync, lstatSync } from 'node:fs';
import path from 'node:path';
import { webPersonalInScope, webCommunityInScope, validateWebOperation, validateWebBrowser, validateWebDirect, validateComposedWebCoverage } from './release_web_controls.mjs';
const [input, output, repository] = process.argv.slice(2);
const sha = file => { if (!lstatSync(file).isFile() || lstatSync(file).isSymbolicLink()) throw new Error('invalid evidence file'); return createHash('sha256').update(readFileSync(file)).digest('hex'); };
const json = file => JSON.parse(readFileSync(file, 'utf8'));
try {
  const inputSha = sha(input), config = json(input), receipt = json(output), root = path.dirname(output);
  assert.equal(config.contract, 'ait.release.web-admission-input/v1');
  const personal = webPersonalInScope(config);
  const community = webCommunityInScope(config);
  const composition = validateWebOperation(input, config, repository, root);
  if (composition) validateComposedWebCoverage(input, config, repository, root);
  if (!personal) assert(repository && path.isAbsolute(repository), 'retained admission requires its bound acceptance repository');
  if (receipt.contract !== 'ait.release.web-admission/v1' || receipt.status !== 'admitted' || receipt.input_sha256 !== inputSha || receipt.release_id !== config.release?.id) throw new Error('Web candidate identity differs');
  for (const step of ['foundation', 'solo_local', 'solo_remote', 'randomized_3000', 'closeout']) {
    // Composition verifies the original foundation against its original input,
    // plus the ordinary action outputs and their native Plan/output digests.
    // Never reattribute that receipt to a newly bound candidate or WT input.
    if (composition && ['foundation', 'solo_local', 'solo_remote', 'randomized_3000'].includes(step)) continue;
    const r = json(path.join(root, 'receipts', `${step}.json`));
    if (r.contract !== 'ait.release.web-admission-step/v1' || r.status !== 'pass' || r.step !== step || r.input_sha256 !== inputSha || sha(r.evidence.path) !== r.evidence.sha256) throw new Error(`Web step evidence drifted: ${step}`);
    if (repository && ['foundation', 'solo_local', 'solo_remote'].includes(step)) {
      validateWebDirect(json(r.evidence.path), config, { foundation: 'inventory', solo_local: 'solo-local', solo_remote: 'solo-remote' }[step], repository, path.dirname(r.evidence.path));
    }
    if (step === 'randomized_3000') {
      const random = json(r.evidence.path);
      assert.equal(random.contract, 'ait-web-test.random-command-run.v1'); assert.equal(random.status, 'pass');
      assert.equal(random.mode, 'solo_remote'); assert.equal(random.repository_index, 23); assert.equal(random.namespace, 'WT');
      assert.equal(config.execution.random_groups, 3000); assert.equal(random.seed, config.execution.random_seed);
      assert.equal(random.requested_groups, 3000); assert.equal(random.executed_groups, 3000); assert.equal(random.retirement_invocations, 0);
      assert.equal(random.initial_authority_digest, random.final_authority_digest);
    }
  }
  for (const mode of ['solo_local', 'solo_remote']) {
    if (community) {
      const file = path.join(root, config.browser_evidence[mode].path);
      validateWebBrowser(json(file), config, mode, repository);
      assert.equal(receipt.browser[`${mode}_sha256`], sha(file));
    }
    assert.equal(receipt.direct[mode], 'pass');
  }
  if (!community) assert.deepEqual(receipt.browser, {}, 'paused companions must not carry a browser pass');
  assert.equal(receipt.randomized.seed, config.execution.random_seed); assert.equal(receipt.randomized.groups, 3000); assert.equal(receipt.randomized.status, 'pass');
} catch (error) { console.error(error.message); process.exit(65); }

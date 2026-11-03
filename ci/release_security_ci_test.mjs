import assert from 'node:assert/strict';
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { spawnSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const read = (name) => readFileSync(path.join(root, name), 'utf8');
const receipt = read('.github/workflows/ait-release-component-receipts.yml');
const qualification = read('.github/workflows/ait-release-pre-rc-qualification.yml');
const quality = read('release/monorepo/.github/workflows/ait-source-quality.yml');
const guardName = 'Require source in the reviewed main history before executing it';
const guardStep = receipt.split(`      - name: ${guardName}\n`)[1]?.split('\n      - name:')[0];
assert.ok(guardStep, 'receipt must admit source before executing source code');
assert.ok(receipt.indexOf(guardName) < receipt.indexOf('Check out the exact public source commit'));
const guard = guardStep.split('        run: |\n')[1].split('\n').map(line => line.slice(10)).join('\n');
assert.ok(receipt.includes('ref: ${{ github.sha }}\n          fetch-depth: 0'));
assert.ok(!qualification.includes('ref: ${{ inputs.source_commit }}'));
assert.ok(qualification.includes('test "${AIT_CONTROL_COMMIT}" = "${AIT_SOURCE_COMMIT}"'));
assert.match(quality, /\n  pull_request:\n/);
assert.match(quality, /branches: \[main\]/);
assert.ok(quality.includes('persist-credentials: false'));
assert.ok(quality.includes('ubuntu-22.04, macos-14, windows-2025'));
for (const required of ['bash ait-core/ci/run.sh patchset', 'bash ait-runner/ci/run.sh all', './ait-core/ci/run.ps1 patchset', './ait-runner/ci/run.ps1 all']) {
  assert.ok(quality.includes(required));
}
for (const forbidden of ['pull_request_target:', 'secrets.', 'contents: write', 'actions/cache@']) {
  assert.ok(!quality.includes(forbidden), `source quality must not gain ${forbidden}`);
}

const temporary = mkdtempSync(path.join(tmpdir(), 'ait-security-ci-'));
function git(...args) {
  const result = spawnSync('git', args, { cwd: temporary, encoding: 'utf8' });
  assert.equal(result.status, 0, result.stderr);
  return result.stdout.trim();
}
function admit(commit, ref = 'refs/heads/main') {
  return spawnSync('bash', ['-c', guard], {
    cwd: temporary, encoding: 'utf8',
    env: { ...process.env, AIT_CONTROL_REF: ref, AIT_SOURCE_GIT_COMMIT: commit },
  }).status;
}
try {
  git('init', '-q', '-b', 'main');
  git('config', 'user.name', 'Security boundary test');
  git('config', 'user.email', 'security-test@localhost');
  writeFileSync(path.join(temporary, 'source'), 'reviewed');
  git('add', '.'); git('commit', '-qm', 'reviewed source');
  const ancestor = git('rev-parse', 'HEAD');
  git('checkout', '-qb', 'unreviewed');
  writeFileSync(path.join(temporary, 'source'), 'unreviewed');
  git('commit', '-qam', 'unreviewed branch');
  const unreviewed = git('rev-parse', 'HEAD');
  git('checkout', '-q', 'main');
  writeFileSync(path.join(temporary, 'control'), 'reviewed control');
  git('add', '.'); git('commit', '-qm', 'reviewed control');
  assert.equal(admit(ancestor), 0, 'reviewed ancestors remain valid receipt sources');
  assert.equal(admit(git('rev-parse', 'HEAD')), 0, 'same-commit receipt source remains valid');
  assert.notEqual(admit(unreviewed), 0, 'unmerged source cannot execute in the main context');
  assert.notEqual(admit(ancestor, 'refs/heads/unreviewed'), 0);
  assert.notEqual(admit('main'), 0, 'mutable refs are refused');
  assert.notEqual(admit('$(exit 0)'), 0, 'input is data, never shell code');
} finally {
  rmSync(temporary, { recursive: true, force: true });
}
console.log('Security CI and dispatch trust contract: pass');

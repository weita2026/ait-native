import { createHash } from 'node:crypto';
import { readFileSync, mkdtempSync, writeFileSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { spawnSync } from 'node:child_process';
const sha = bytes => createHash('sha256').update(bytes).digest('hex');
const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const controlFiles = ['ci/release_winget_bootstrap.ps1','ci/release_winget_pre_review.ps1','.github/workflows/ait-release-winget-pre-review.yml'];
function requireThat(value, message) { if (!value) throw new Error(`WinGet pre-review gate: ${message}`); }
export function validatePreReviewReceipt(receipt, expected) {
  requireThat(receipt.contract === 'ait.release.winget-pre-review/v1' && receipt.status === 'installed_verified', 'installation is not verified');
  for (const key of ['version','release_id','architecture','control_commit','bootstrap_sha256']) requireThat(receipt[key] === expected[key], `${key} differs`);
  requireThat(receipt.fresh_host === true && receipt.scope === 'user', 'fresh user install is required');
  requireThat(Object.keys(receipt.manifests ?? {}).length === expected.files.length, 'manifest inventory differs');
  for (const file of expected.files) requireThat(receipt.manifests?.[file.name] === file.sha256, `manifest hash differs: ${file.name}`);
  requireThat(receipt.commands?.length === 3, 'three installed commands are required');
  for (const name of ['ait','ait-server','ait-runner']) {
    const rows = receipt.commands.filter(row => row.name === name);
    requireThat(rows.length === 1 && rows[0].reported_version === `${name} ${expected.version}` && /^[a-f0-9]{64}$/.test(rows[0].sha256), `installed command differs: ${name}`);
  }
}
export function verifyPreReview({runId, releaseId, repository, version, files}) {
  requireThat(/^[1-9][0-9]*$/.test(runId ?? ''), 'a successful --pre-review-run-id is required before submission');
  requireThat(/^REL-FAM-[0-9A-F]{16}$(?![\s\S])/.test(releaseId ?? ''), '--release-id is required before submission');
  const api = endpoint => {
    const result = spawnSync('gh', ['api', endpoint], {maxBuffer: 32 * 1024 * 1024});
    requireThat(result.status === 0, `cannot read ${endpoint}`);
    return result.stdout;
  };
  const json = endpoint => JSON.parse(api(endpoint));
  const run = json(`repos/${repository}/actions/runs/${runId}`);
  requireThat(String(run.id) === runId && run.status === 'completed' && run.conclusion === 'success' && run.event === 'workflow_dispatch' && run.path === '.github/workflows/ait-release-winget-pre-review.yml' && /^[a-f0-9]{40}$/.test(run.head_sha), 'exact pre-review workflow must succeed');
  for (const file of controlFiles) {
    const remote = json(`repos/${repository}/contents/${file}?ref=${run.head_sha}`);
    requireThat(Buffer.from(remote.content ?? '', 'base64').equals(readFileSync(path.join(root,file))), `tested control differs: ${file}`);
  }
  const artifacts = json(`repos/${repository}/actions/runs/${runId}/artifacts?per_page=100`).artifacts;
  const records = [];
  const temp = mkdtempSync(path.join(tmpdir(), 'ait-winget-pre-review-'));
  try {
    for (const architecture of ['x64','arm64']) {
      const matches = artifacts.filter(a => a.name === `ait-winget-pre-review-${version}-${architecture}` && !a.expired);
      requireThat(matches.length === 1, `one ${architecture} artifact is required`);
      const artifact = matches[0];
      const bytes = api(`repos/${repository}/actions/artifacts/${artifact.id}/zip`);
      requireThat(artifact.digest === `sha256:${sha(bytes)}`, `${architecture} artifact digest differs`);
      const zip = path.join(temp, `${architecture}.zip`); writeFileSync(zip,bytes);
      const result = spawnSync('unzip',['-p',zip,'winget-pre-review.json'],{encoding:'utf8'});
      requireThat(result.status === 0, `${architecture} receipt missing`);
      const receipt = JSON.parse(result.stdout.replace(/^\uFEFF/,''));
      validatePreReviewReceipt(receipt,{version,release_id:releaseId,architecture,control_commit:run.head_sha,bootstrap_sha256:sha(readFileSync(path.join(root,controlFiles[0]))),files});
      records.push({architecture,artifact_id:artifact.id,artifact_digest:artifact.digest,receipt_sha256:sha(Buffer.from(result.stdout))});
    }
  } finally { rmSync(temp,{recursive:true,force:true}); }
  return {workflow_run_id:run.id,workflow_run_attempt:run.run_attempt,control_commit:run.head_sha,release_id:releaseId,status:'passed',artifacts:records};
}

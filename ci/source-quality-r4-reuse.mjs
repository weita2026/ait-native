import assert from 'node:assert/strict';
import { pathToFileURL } from 'node:url';

const repository = 'weita2026/ait-native';
const runId = 41544513498, attempt = 1;
const head = 'c1f64918e6f2360f2029696f554e32760acc310d';
const branch = 'diagnostic/release-121-20261103-r4';
const workflow = 'AIT source quality';
const retained = Object.freeze({
  Linux: Object.freeze({ id: 125616699768, label: 'ubuntu-22.04', conclusion: 'success', core: 'success' }),
  macOS: Object.freeze({ id: 125616700080, label: 'macos-14', conclusion: 'failure', core: 'failure' }),
});

export function validateRetainedR4Runner(run, job, logs, os) {
  const expected = retained[os]; assert(expected, 'R4 Runner reuse is limited to Linux and macOS');
  for (const [key, value] of Object.entries({ id: runId, run_attempt: attempt, head_sha: head, head_branch: branch,
    path: '.github/workflows/ait-source-quality.yml', name: workflow, event: 'workflow_dispatch', status: 'completed', conclusion: 'failure' })) {
    assert.equal(run?.[key], value, `R4 run ${key} differs`);
  }
  assert.equal(run.repository?.full_name, repository); assert.equal(run.head_repository?.full_name, repository);
  for (const [key, value] of Object.entries({ id: expected.id, run_id: runId, run_attempt: attempt, head_sha: head,
    head_branch: branch, workflow_name: workflow, name: `Locked Core and Runner / ${expected.label}`,
    status: 'completed', conclusion: expected.conclusion })) assert.equal(job?.[key], value, `R4 job ${key} differs`);
  assert.deepEqual(job.labels, [expected.label]);
  const steps = job.steps?.filter(step => step.name === 'Runner gates (Unix)');
  assert.equal(steps?.length, 1, 'R4 Unix Runner step is missing or ambiguous');
  assert.equal(steps[0].status, 'completed'); assert.equal(steps[0].conclusion, 'success');
  // continue-on-error can normalize the REST step conclusion. The original
  // final gate printed steps.*.outcome, so require that original stream too.
  assert.equal(typeof logs, 'string');
  const lines = logs.split(/\r?\n/).map(line => line.replace(/^\d{4}-\d\d-\d\dT\d\d:\d\d:\d\d\.\d+Z /, ''));
  assert.deepEqual(lines.filter(line => line.startsWith('Runner: ')), ['Runner: success'], 'R4 Runner outcome is not proven');
  assert.deepEqual(lines.filter(line => line.startsWith('Core: ')), [`Core: ${expected.core}`], 'R4 Core outcome differs');
  return `Runner: reused R4 run ${runId}/attempt ${attempt}, head ${head}, job ${expected.id}, successful Unix Runner step; not executed in this batch`;
}

export async function requireDiagnosticR4Results(context, api) {
  assert.equal(context.event, 'workflow_dispatch', 'R4 reuse requires an explicit diagnostic dispatch');
  assert.equal(context.repository, repository);
  assert(context.ref?.startsWith('refs/heads/diagnostic/release-121-'), 'R4 reuse requires the bounded diagnostic branch');
  assert.notEqual(context.ref, `refs/heads/${branch}`, 'the original R4 branch must remain immutable');
  assert(['Linux', 'macOS', 'Windows'].includes(context.os));
  const errors = [], output = [`Core: fresh ${context.core}`];
  const check = callback => { try { callback(); } catch (error) { errors.push(error.message); } };
  check(() => assert.equal(context.core, 'success', 'fresh Core gate did not pass'));
  if (context.os === 'Windows') {
    output.push(`Runner: fresh ${context.runner}`);
    check(() => assert.equal(context.runner, 'success', 'fresh Windows Runner gate did not pass'));
  } else {
    check(() => assert.equal(context.runner, 'skipped', 'reused Unix Runner must not execute in this batch'));
    // Retained evidence is independent of a fresh Core failure. Collect it
    // before failing the aggregate rather than hiding a second failure.
    const base = `/repos/${repository}/actions`;
    const expected = retained[context.os];
    const responses = await Promise.allSettled([
      api.json(`${base}/runs/${runId}/attempts/${attempt}`),
      api.json(`${base}/jobs/${expected.id}`),
      api.text(`${base}/jobs/${expected.id}/logs`),
    ]);
    for (const result of responses) if (result.status === 'rejected') errors.push(result.reason?.message ?? String(result.reason));
    if (responses.every(result => result.status === 'fulfilled')) {
      check(() => output.push(validateRetainedR4Runner(...responses.map(result => result.value), context.os)));
    }
  }
  if (errors.length) throw new Error([...output, ...errors].join('\n'));
  return [...output, 'Diagnostic affected inventory only; retained R4 results are not fresh Runner results.'];
}

async function main() {
  const request = async (resource, json) => {
    assert(process.env.GH_TOKEN, 'read-only Actions token is unavailable');
    const response = await fetch(`https://api.github.com${resource}`, { headers: {
      authorization: `Bearer ${process.env.GH_TOKEN}`, accept: 'application/vnd.github+json', 'X-GitHub-Api-Version': '2022-11-28',
    }, signal: AbortSignal.timeout(30_000), redirect: json ? 'error' : 'follow' });
    assert(response.ok, `R4 evidence request failed (${response.status}): ${resource}`);
    const body = await response.text(); assert(Buffer.byteLength(body) <= 32 * 1024 * 1024, 'R4 evidence exceeds its bounded size');
    return json ? JSON.parse(body) : body;
  };
  const lines = await requireDiagnosticR4Results({ event: process.env.SOURCE_QUALITY_EVENT,
    repository: process.env.SOURCE_QUALITY_REPOSITORY, ref: process.env.SOURCE_QUALITY_REF, os: process.env.RUNNER_OS,
    core: process.env.CORE_RESULT, runner: process.env.RUNNER_RESULT },
  { json: resource => request(resource, true), text: resource => request(resource, false) });
  console.log(lines.join('\n'));
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  main().catch(error => { console.error(error.message); process.exitCode = 1; });
}

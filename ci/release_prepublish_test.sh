#!/usr/bin/env bash
set -euo pipefail

repo_root=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
verifier=${repo_root}/ci/release_prepublish_verify.mjs
stage_control=${repo_root}/ci/release_prepublish_stage.sh
oci_control=${repo_root}/ci/release_prepublish_oci.sh
promotion_control=${repo_root}/ci/release_candidate_promote.sh
prepublish_workflow=${repo_root}/.github/workflows/ait-release-prepublish-clean-host.yml
protected_workflow=${repo_root}/.github/workflows/ait-release-protected-promotion.yml
publication_workflow=${repo_root}/.github/workflows/pypi-publish.yml
temporary_root=$(mktemp -d "${TMPDIR:-/tmp}/ait-release-prepublish-test.XXXXXX")

cleanup() {
  case "${temporary_root}" in
    "${TMPDIR:-/tmp}"/ait-release-prepublish-test.*) rm -rf -- "${temporary_root}" ;;
    *) printf 'refusing to remove unexpected prepublish test path: %s\n' \
      "${temporary_root}" >&2 ;;
  esac
}
trap cleanup EXIT HUP INT TERM

sha256_file() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1" | awk '{print $1}'
  else
    shasum -a 256 "$1" | awk '{print $1}'
  fi
}

write_inventory() {
  local root=$1
  local name=$2
  find "${root}" -type f ! -name "${name}" -print | LC_ALL=C sort |
    while IFS= read -r file; do
      printf '%s  %s\n' "$(sha256_file "${file}")" "${file#"${root}/"}"
    done >"${root}/${name}"
}

expect_failure() {
  local label=$1
  shift
  if "$@" >"${temporary_root}/${label}.stdout" \
    2>"${temporary_root}/${label}.stderr"; then
    printf 'expected prepublish failure: %s\n' "${label}" >&2
    return 1
  fi
  test -s "${temporary_root}/${label}.stderr"
}

require_direct_exact_artifact_downloads() {
  local workflow=$1
  local line
  local block
  while IFS=: read -r line _; do
    block=$(sed -n "${line},$((line + 5))p" "${workflow}")
    if ! grep -F 'merge-multiple: true' <<<"${block}" >/dev/null; then
      printf 'exact artifact download does not extract into its consumer root: %s:%s\n' \
        "${workflow}" "${line}" >&2
      return 1
    fi
  done < <(grep -n '^[[:space:]]*artifact-ids:' "${workflow}")
}

node --check "${verifier}"
bash -n "${stage_control}"
bash -n "${oci_control}"
bash -n "${promotion_control}"
for required in \
  'permissions:' \
  'actions: read' \
  'contents: read' \
  'candidate_authority_sha256:' \
  'candidate_authority_b64:' \
  'git -c credential.helper= ls-remote --tags' \
  'release_prepublish_stage.sh --pre-tag' \
  'AIT_NEW_ARTIFACT_DIGEST: ${{ steps.upload.outputs.artifact-digest }}' \
  'if [[ ${candidate_artifact_digest} =~ ^[0-9a-f]{64}$ ]]; then' \
  'candidate_artifact_digest=sha256:${candidate_artifact_digest}' \
  '[[ "${candidate_artifact_digest}" =~ ^sha256:[0-9a-f]{64}$ ]]' \
  'Recheck tag absence at qualification closeout' \
  'release_prepublish_verify.mjs qualify' \
  'ait-pre-tag-clean-host-${{ inputs.release_id }}'; do
  grep -F -- "${required}" "${prepublish_workflow}" >/dev/null
done
grep -F 'environment:' "${publication_workflow}" >/dev/null
for required in \
  'Download the exact pre-tag candidate' \
  'Download the successful pre-tag qualification' \
  'artifact-ids: ${{ steps.request.outputs.candidate_artifact_id }}' \
  'artifact-ids: ${{ steps.request.outputs.aggregate_artifact_id }}' \
  'release_candidate_promote.sh'; do
  grep -F -- "${required}" "${publication_workflow}" >/dev/null
done
if grep -F 'needs: prepublish' "${publication_workflow}" >/dev/null; then
  printf 'publication workflow still rebuilds or requalifies after tag creation\n' >&2
  exit 65
fi
require_direct_exact_artifact_downloads "${prepublish_workflow}"
require_direct_exact_artifact_downloads "${protected_workflow}"
require_direct_exact_artifact_downloads "${publication_workflow}"
if grep -E '(^|[[:space:]])(gh release|npm publish|docker push)([[:space:]]|$)' \
  "${prepublish_workflow}" "${stage_control}" >/dev/null; then
  printf 'prepublish controls contain a public endpoint write\n' >&2
  exit 65
fi

candidate=${temporary_root}/candidate
mkdir -p "${candidate}/assets" "${candidate}/oci-archives"
config=${candidate}/ait-release.endpoints.authority.json
jq -S -n '{
  contract: "ait.release.family.endpoints/v1",
  release: {
    id: "REL-FAM-0123456789ABCDEF",
    version: "1.2.3-rc.6",
    python_version: "1.2.3rc6",
    channel: "rc",
    tag: "v1.2.3-rc.6",
    source_commit: "1111111111111111111111111111111111111111"
  }
}' >"${config}"
printf 'endpoint-stage\n' >"${candidate}/ait-release.endpoint-publication.json"
debian_asset=${candidate}/assets/ait-native_1.2.3~rc.6_amd64.deb
printf 'debian-rc-package\n' >"${debian_asset}"
printf '%s  %s\n' "$(sha256_file "${debian_asset}")" \
  "$(basename "${debian_asset}")" >"${candidate}/assets/SHA256SUMS"
for component in ait-server ait-runner; do
  for architecture in amd64 arm64; do
    printf '%s-%s\n' "${component}" "${architecture}" \
      >"${candidate}/oci-archives/${component}-${architecture}.docker.tar"
  done
done

oci=${temporary_root}/oci.json
jq -S -n --arg root "${candidate}" '
  reduce ["ait-server", "ait-runner"][] as $component ({};
    reduce ["amd64", "arm64"][] as $architecture (.;
      ($component + "-" + $architecture + ".docker.tar") as $archive |
      .[$component][$architecture] = {
        archive: $archive,
        sha256: null,
        reference: ("ait-prepublish/" + $component + ":1.2.3-rc.6-" + $architecture),
        image_id: ("sha256:" + (if $component == "ait-server" then
          (if $architecture == "amd64" then "2" else "3" end)
        else
          (if $architecture == "amd64" then "4" else "5" end)
        end) * 64)
      }
    ))
' >"${oci}"
for component in ait-server ait-runner; do
  for architecture in amd64 arm64; do
    archive=${component}-${architecture}.docker.tar
    jq --arg component "${component}" --arg architecture "${architecture}" \
      --arg digest "$(sha256_file "${candidate}/oci-archives/${archive}")" \
      '.[$component][$architecture].sha256 = $digest' "${oci}" >"${oci}.new"
    mv "${oci}.new" "${oci}"
  done
done

receipt=${candidate}/ait-release.prepublish-stage.json
jq -S -n --slurpfile oci "${oci}" \
  --arg config_sha "$(sha256_file "${config}")" \
  --arg endpoint_sha "$(sha256_file "${candidate}/ait-release.endpoint-publication.json")" \
  --arg assets_sha "$(sha256_file "${candidate}/assets/SHA256SUMS")" '{
  contract: "ait.release.prepublish.stage/v1",
  status: "frozen_candidate_staged",
  release: {
    id: "REL-FAM-0123456789ABCDEF",
    version: "1.2.3-rc.6",
    tag: "v1.2.3-rc.6",
    source_commit: "1111111111111111111111111111111111111111"
  },
  qualification_stage: "post_authorization",
  tag_state: "present",
  authority: {
    endpoint_config_sha256: $config_sha,
    endpoint_stage_receipt_sha256: $endpoint_sha,
    assets_checksum_sha256: $assets_sha
  },
  oci: $oci[0],
  mutation: {
    artifact_rebuild: false,
    component_rebuild: false,
    registry_write: false,
    endpoint_write: false,
    github_release_write: false,
    tag_write: false,
    service_start: false
  }
}' >"${receipt}"
status=${candidate}/ait-release.prepublish-candidate.json
jq -S -n --slurpfile oci "${oci}" \
  --arg receipt_sha "$(sha256_file "${receipt}")" '{
  contract: "ait.release.prepublish.candidate/v1",
  status: "frozen_candidate_pending_clean_host",
  release: {
    id: "REL-FAM-0123456789ABCDEF",
    version: "1.2.3-rc.6",
    tag: "v1.2.3-rc.6",
    source_commit: "1111111111111111111111111111111111111111"
  },
  qualification_stage: "post_authorization",
  tag_state: "present",
  candidate: {stage_receipt_sha256: $receipt_sha, oci: $oci[0]},
  public_endpoint_writes: false
}' >"${status}"
write_inventory "${candidate}" PREPUBLISH_SHA256SUMS
config_sha=$(sha256_file "${config}")
status_sha=$(sha256_file "${status}")
node "${verifier}" stage --root "${candidate}" \
  --config-sha256 "${config_sha}" --status-sha256 "${status_sha}" >/dev/null

node --input-type=module - "${repo_root}" "${temporary_root}" "${candidate}" <<'DOWNLOAD_ACTION_TEST'
import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { spawn, spawnSync } from 'node:child_process';
import { once } from 'node:events';
import { existsSync, readFileSync, readdirSync, writeFileSync } from 'node:fs';
import http from 'node:http';
import path from 'node:path';

const [root, temporary, candidate] = process.argv.slice(2);
const fixedPin = '018cc2cf5baa6db3ef3c5f8a56943fffe632ef53';
const oldPin = 'd3f86a106a0bac45b974a628896c90dbdf5c8093';
const bundleHashes = {
  [fixedPin]: '080f0467d57952d7adb6b16a27b43c546a99609736bba5c799ff1d226007c06b',
  [oldPin]: '02c2cf24c5b60ae4b3f8d71d3cc85e2e6433dbdf106a38fee7ec3f1c96e2d18e',
};
const sha = bytes => createHash('sha256').update(bytes).digest('hex');
for (const [name, downloads, exact] of [
  ['ait-release-prepublish-clean-host.yml', 6, 4],
  ['ait-release-protected-promotion.yml', 3, 3],
  ['pypi-publish.yml', 3, 3],
]) {
  const source = readFileSync(path.join(root, '.github/workflows', name), 'utf8');
  const steps = source.split(/^      - name: /m).filter(step => step.includes('uses: actions/download-artifact@'));
  assert.equal(steps.length, downloads, `${name}: download step count`);
  assert(steps.every(step => step.includes(`uses: actions/download-artifact@${fixedPin} # v6.0.0`)));
  const byId = steps.filter(step => step.includes('artifact-ids:'));
  assert.equal(byId.length, exact, `${name}: exact artifact step count`);
  for (const step of byId) {
    assert.match(step, /artifact-ids: \$\{\{ [^\n]+ \}\}/);
    assert.match(step, /run-id: \$\{\{ [^\n]+ \}\}/);
    assert(step.includes('repository: ${{ github.repository }}'));
    assert(step.includes('github-token: ${{ github.token }}'));
    assert(step.includes('merge-multiple: true'));
    assert.match(step, /path: \S+/);
  }
}

// Execute the unmodified, digest-pinned action bundles against a loopback API.
const bundles = new Map(await Promise.all([oldPin, fixedPin].map(async pin => {
  const response = await fetch(`https://raw.githubusercontent.com/actions/download-artifact/${pin}/dist/index.js`,
    { signal: AbortSignal.timeout(60_000) });
  assert(response.ok, `reviewed action bundle ${pin}: HTTP ${response.status}`);
  const bytes = Buffer.from(await response.arrayBuffer());
  assert.equal(sha(bytes), bundleHashes[pin], `reviewed action bundle drifted: ${pin}`);
  const file = path.join(temporary, `${pin}.cjs`);
  writeFileSync(file, bytes);
  return [pin, file];
})));

const files = directory => readdirSync(directory, { withFileTypes: true }).flatMap(entry =>
  entry.isDirectory() ? files(path.join(directory, entry.name)) : [path.join(directory, entry.name)]);
const entries = files(candidate).sort().map(file => [path.relative(candidate, file).split(path.sep).join('/'), readFileSync(file)]);
const crcTable = Array.from({ length: 256 }, (_, byte) => {
  let value = byte;
  for (let bit = 0; bit < 8; bit++) value = (value >>> 1) ^ ((value & 1) ? 0xedb88320 : 0);
  return value >>> 0;
});
const crc32 = bytes => {
  let value = 0xffffffff;
  for (const byte of bytes) value = (value >>> 8) ^ crcTable[(value ^ byte) & 0xff];
  return (value ^ 0xffffffff) >>> 0;
};
const zip = records => {
  const local = [], central = [];
  let offset = 0;
  for (const [name, bytes] of records) {
    const filename = Buffer.from(name), crc = crc32(bytes), header = Buffer.alloc(30), index = Buffer.alloc(46);
    header.writeUInt32LE(0x04034b50); header.writeUInt16LE(20, 4); header.writeUInt16LE(0x800, 6);
    header.writeUInt16LE(0x21, 12); header.writeUInt32LE(crc, 14);
    header.writeUInt32LE(bytes.length, 18); header.writeUInt32LE(bytes.length, 22); header.writeUInt16LE(filename.length, 26);
    index.writeUInt32LE(0x02014b50); index.writeUInt16LE(20, 4); index.writeUInt16LE(20, 6);
    index.writeUInt16LE(0x800, 8); index.writeUInt16LE(0x21, 14); index.writeUInt32LE(crc, 16);
    index.writeUInt32LE(bytes.length, 20); index.writeUInt32LE(bytes.length, 24); index.writeUInt16LE(filename.length, 28);
    index.writeUInt32LE(offset, 42);
    local.push(header, filename, bytes); central.push(index, filename);
    offset += header.length + filename.length + bytes.length;
  }
  const directory = Buffer.concat(central), end = Buffer.alloc(22);
  end.writeUInt32LE(0x06054b50); end.writeUInt16LE(records.length, 8); end.writeUInt16LE(records.length, 10);
  end.writeUInt32LE(directory.length, 12); end.writeUInt32LE(offset, 16);
  return Buffer.concat([...local, directory, end]);
};
const validZip = zip(entries);
const corruptZip = zip(entries.map(([name, bytes]) => [name,
  name.endsWith('.deb') ? Buffer.concat([bytes, Buffer.from('tamper')]) : bytes]));
const artifactId = 9001, runId = '4242', repository = 'fixture-owner/fixture-repo';
const artifactName = 'ait-pre-tag-candidate-fixture', token = 'fixture-artifact-token';
const state = { calls: [], payload: validZip };
let base;
const server = http.createServer((request, response) => {
  const url = new URL(request.url, base);
  const reply = (status, value) => {
    const bytes = Buffer.from(JSON.stringify(value));
    response.writeHead(status, { 'content-type': 'application/json', 'content-length': bytes.length }); response.end(bytes);
  };
  if (request.method !== 'GET') return reply(405, { message: 'fixture requires GET' });
  if (url.pathname === '/blob/candidate.zip') {
    state.calls.push({ kind: 'blob' });
    response.writeHead(200, { 'content-type': 'application/zip', 'content-length': state.payload.length });
    response.end(state.payload); return;
  }
  if (![`token ${token}`, `Bearer ${token}`].includes(request.headers.authorization)) {
    state.calls.push({ kind: 'denied-token' }); return reply(401, { message: 'fixture token mismatch' });
  }
  if (url.pathname === `/repos/${repository}/actions/runs/${runId}/artifacts`) {
    const page = Number(url.searchParams.get('page') ?? '1');
    if (url.searchParams.get('per_page') !== '100') return reply(400, { message: 'fixture requires 100 artifacts per page' });
    state.calls.push({ kind: 'list', page });
    const artifacts = Array.from({ length: 101 }, (_, index) => ({
      id: index === 100 ? artifactId : 1000 + index,
      name: index === 100 ? artifactName : `previous-${index % 67}`,
      size_in_bytes: state.payload.length, expired: false,
      digest: `sha256:${sha(state.payload)}`,
      created_at: new Date(Date.UTC(2026, 0, 1, 0, 0, 101 - index)).toISOString(),
    }));
    reply(200, { total_count: 101, artifacts: artifacts.slice((page - 1) * 100, page * 100) }); return;
  }
  if (url.pathname === `/repos/${repository}/actions/artifacts/${artifactId}/zip`) {
    state.calls.push({ kind: 'download', id: artifactId });
    response.writeHead(302, { location: `${base}/blob/candidate.zip` }); response.end(); return;
  }
  state.calls.push({ kind: 'denied-scope' }); reply(404, { message: 'fixture repository/run/artifact mismatch' });
});
server.listen(0, '127.0.0.1'); await once(server, 'listening');
base = `http://127.0.0.1:${server.address().port}`;
const action = async (label, pin, inputs = {}, payload = validZip) => {
  state.calls = []; state.payload = payload;
  const destination = path.join(temporary, `download-${label}`), output = path.join(temporary, `${label}.outputs`);
  writeFileSync(output, '');
  const env = { ...process.env };
  for (const key of Object.keys(env)) if (key.startsWith('INPUT_') || /^(HTTP|HTTPS|ALL)_PROXY$/i.test(key)) delete env[key];
  for (const key of ['ACTIONS_RUNTIME_TOKEN', 'ACTIONS_RESULTS_URL', 'ACTIONS_RUNTIME_URL', 'NODE_OPTIONS']) delete env[key];
  Object.assign(env, { GITHUB_API_URL: base, GITHUB_SERVER_URL: 'https://github.com', GITHUB_OUTPUT: output,
    GITHUB_WORKSPACE: temporary, GITHUB_REPOSITORY: 'other/host-context', GITHUB_RUN_ID: '999999',
    GITHUB_TOKEN: 'unused-default-token', GH_TOKEN: 'unused-default-token',
    'INPUT_ARTIFACT-IDS': String(artifactId), 'INPUT_RUN-ID': runId, INPUT_REPOSITORY: repository,
    'INPUT_GITHUB-TOKEN': token, INPUT_PATH: destination, 'INPUT_MERGE-MULTIPLE': 'true', ...inputs });
  const child = spawn(process.execPath, [bundles.get(pin)], { env, stdio: ['ignore', 'pipe', 'pipe'] });
  let stdout = '', stderr = '', timedOut = false;
  child.stdout.on('data', data => { stdout += data; }); child.stderr.on('data', data => { stderr += data; });
  const timer = setTimeout(() => { timedOut = true; child.kill('SIGKILL'); }, 45_000);
  let status;
  try { [status] = await once(child, 'close'); } finally { clearTimeout(timer); }
  assert(!timedOut, `${label}: actual action timed out\n${stdout}\n${stderr}`);
  return { status, stdout, stderr, destination, calls: [...state.calls] };
};
const stage = destination => spawnSync(process.execPath,
  [path.join(root, 'ci/release_prepublish_verify.mjs'), 'stage', '--root', destination,
    '--config-sha256', sha(readFileSync(path.join(candidate, 'ait-release.endpoints.authority.json'))),
    '--status-sha256', sha(readFileSync(path.join(candidate, 'ait-release.prepublish-candidate.json')))], { encoding: 'utf8' });
try {
  const old = await action('old-page2', oldPin);
  assert.notEqual(old.status, 0); assert.match(old.stdout + old.stderr, /None of the provided artifact IDs were found/);
  assert.deepEqual(old.calls.filter(call => call.kind === 'list').map(call => call.page), [1]);
  const fixed = await action('fixed-page2', fixedPin);
  assert.equal(fixed.status, 0, fixed.stdout + fixed.stderr);
  assert.deepEqual(fixed.calls.filter(call => call.kind === 'list').map(call => call.page), [1, 2]);
  assert.deepEqual(fixed.calls.filter(call => call.kind === 'download').map(call => call.id), [artifactId]);
  assert(fixed.calls.some(call => call.kind === 'blob'));
  assert(existsSync(path.join(fixed.destination, 'ait-release.endpoints.authority.json')));
  assert(!existsSync(path.join(fixed.destination, artifactName)));
  assert.equal(stage(fixed.destination).status, 0);
  for (const [label, inputs, rejection] of [
    ['missing-id', { 'INPUT_ARTIFACT-IDS': '99999' }, 'list'],
    ['wrong-run', { 'INPUT_RUN-ID': '4243' }, 'denied-scope'],
    ['wrong-repository', { INPUT_REPOSITORY: 'other/repository' }, 'denied-scope'],
    ['wrong-token', { 'INPUT_GITHUB-TOKEN': 'wrong-token' }, 'denied-token'],
  ]) {
    const result = await action(label, fixedPin, inputs);
    assert.notEqual(result.status, 0, label); assert(result.calls.some(call => call.kind === rejection), label);
    assert(!result.calls.some(call => call.kind === 'blob'), label);
  }
  const corrupt = await action('corrupt-content', fixedPin, {}, corruptZip);
  assert.equal(corrupt.status, 0, corrupt.stdout + corrupt.stderr);
  const corruptCheck = stage(corrupt.destination);
  assert.notEqual(corruptCheck.status, 0); assert.match(corruptCheck.stderr, /digest|checksum/i);
  const nested = await action('wrong-extraction-root', fixedPin,
    { INPUT_PATH: path.join(temporary, 'download-wrong-extraction-root', 'unexpected-root') });
  assert.equal(nested.status, 0, nested.stdout + nested.stderr);
  assert(existsSync(path.join(nested.destination, 'unexpected-root', 'ait-release.endpoints.authority.json')));
  assert.notEqual(stage(nested.destination).status, 0);
  console.log('reviewed download action: old pin rejects page2; v6 retrieves exact index100 via pages1–2');
  console.log('reviewed download action: exact ID/run/repository/token, content digest and extraction-root guards pass');
} finally {
  server.closeAllConnections(); await new Promise(resolve => server.close(resolve));
}
DOWNLOAD_ACTION_TEST

unsafe_inventory=${temporary_root}/unsafe-inventory
cp -R "${candidate}" "${unsafe_inventory}"
{
  printf '%064d  assets/../escape\n' 0
  cat "${candidate}/PREPUBLISH_SHA256SUMS"
} >"${unsafe_inventory}/PREPUBLISH_SHA256SUMS"
expect_failure unsafe-inventory node "${verifier}" stage \
  --root "${unsafe_inventory}" \
  --config-sha256 "${config_sha}" \
  --status-sha256 "${status_sha}"
grep -F 'prepublish checksum inventory contains an unsafe or duplicate row' \
  "${temporary_root}/unsafe-inventory.stderr" >/dev/null

incomplete_mutation=${temporary_root}/incomplete-mutation
cp -R "${candidate}" "${incomplete_mutation}"
jq 'del(.mutation.tag_write)' "${incomplete_mutation}/ait-release.prepublish-stage.json" \
  >"${incomplete_mutation}/ait-release.prepublish-stage.json.new"
mv "${incomplete_mutation}/ait-release.prepublish-stage.json.new" \
  "${incomplete_mutation}/ait-release.prepublish-stage.json"
jq --arg receipt_sha \
  "$(sha256_file "${incomplete_mutation}/ait-release.prepublish-stage.json")" \
  '.candidate.stage_receipt_sha256 = $receipt_sha' \
  "${incomplete_mutation}/ait-release.prepublish-candidate.json" \
  >"${incomplete_mutation}/ait-release.prepublish-candidate.json.new"
mv "${incomplete_mutation}/ait-release.prepublish-candidate.json.new" \
  "${incomplete_mutation}/ait-release.prepublish-candidate.json"
write_inventory "${incomplete_mutation}" PREPUBLISH_SHA256SUMS
expect_failure incomplete-mutation node "${verifier}" stage \
  --root "${incomplete_mutation}" \
  --config-sha256 "${config_sha}" \
  --status-sha256 \
    "$(sha256_file "${incomplete_mutation}/ait-release.prepublish-candidate.json")"

printf 'tamper\n' >>"${candidate}/oci-archives/ait-server-amd64.docker.tar"
expect_failure stage-drift node "${verifier}" stage --root "${candidate}" \
  --config-sha256 "${config_sha}" --status-sha256 "${status_sha}"
printf 'ait-server-amd64\n' >"${candidate}/oci-archives/ait-server-amd64.docker.tar"

qualification=${temporary_root}/qualification
mkdir -p "${qualification}/rows"
for ordinal in $(seq 1 32); do
  printf '{"row":%s}\n' "${ordinal}" \
    >"${qualification}/rows/row-$(printf '%02d' "${ordinal}").json"
done
cp "${status}" "${qualification}/ait-release.prepublish-candidate.json"
artifact_digest=sha256:9999999999999999999999999999999999999999999999999999999999999999
jq -S -n \
  --slurpfile candidate_config "${config}" \
  --arg artifact_digest "${artifact_digest}" \
  --arg status_sha "${status_sha}" \
  --arg receipt_sha "$(sha256_file "${receipt}")" '{
  contract: "ait.release.clean-host.aggregate/v1",
  status: "qualified",
  release: ($candidate_config[0].release + {
    verification_stage: "prepublication",
    candidate_artifact_digest: $artifact_digest,
    candidate_stage_receipt_sha256: $receipt_sha,
    operator_status_sha256: $status_sha
  }),
  matrix: {expected_rows: 32, admitted_rows: 32, evidence_files: 32},
  failures: [],
  promotion: {allowed: true, retry_same_candidate: false, terminal_for_release: false}
}' >"${qualification}/ait-release.clean-host-status.json"
write_inventory "${qualification}" SHA256SUMS
aggregate_sha=$(sha256_file "${qualification}/ait-release.clean-host-status.json")
node "${verifier}" qualify --root "${qualification}" --candidate-root "${candidate}" \
  --candidate-artifact-digest "${artifact_digest}" \
  --aggregate-sha256 "${aggregate_sha}" >/dev/null

expect_failure wrong-candidate-artifact-digest node "${verifier}" qualify \
  --root "${qualification}" --candidate-root "${candidate}" \
  --candidate-artifact-digest sha256:8888888888888888888888888888888888888888888888888888888888888888 \
  --aggregate-sha256 "${aggregate_sha}"

jq '.matrix.admitted_rows = 31' "${qualification}/ait-release.clean-host-status.json" \
  >"${qualification}/ait-release.clean-host-status.json.new"
mv "${qualification}/ait-release.clean-host-status.json.new" \
  "${qualification}/ait-release.clean-host-status.json"
write_inventory "${qualification}" SHA256SUMS
aggregate_sha=$(sha256_file "${qualification}/ait-release.clean-host-status.json")
expect_failure incomplete-qualification node "${verifier}" qualify \
  --root "${qualification}" --candidate-root "${candidate}" \
  --candidate-artifact-digest "${artifact_digest}" \
  --aggregate-sha256 "${aggregate_sha}"

node --input-type=module - "${repo_root}" "${temporary_root}" "${candidate}" <<'SCOPED_QUALIFICATION_TEST'
import assert from 'node:assert/strict';
import { cpSync, mkdirSync, readFileSync, readdirSync, writeFileSync } from 'node:fs';
import { createHash } from 'node:crypto';
import { spawnSync } from 'node:child_process';
import path from 'node:path';
const [root, temporary, original] = process.argv.slice(2);
const candidate = path.join(temporary, 'candidate-120'); cpSync(original, candidate, { recursive: true });
const read = file => JSON.parse(readFileSync(file, 'utf8'));
const json = (file, value) => writeFileSync(file, JSON.stringify(value));
const sha = file => createHash('sha256').update(readFileSync(file)).digest('hex');
const list = directory => readdirSync(directory, { withFileTypes: true }).flatMap(entry => entry.isDirectory()
  ? list(path.join(directory, entry.name)) : [path.join(directory, entry.name)]);
const inventory = directory => writeFileSync(path.join(directory, 'SHA256SUMS'), list(directory)
  .filter(file => path.basename(file) !== 'SHA256SUMS').sort()
  .map(file => `${sha(file)}  ${path.relative(directory, file)}\n`).join(''));
const configFile = path.join(candidate, 'ait-release.endpoints.authority.json');
const statusFile = path.join(candidate, 'ait-release.prepublish-candidate.json');
const config = read(configFile), status = read(statusFile);
Object.assign(config.release, { version: '1.2.0', python_version: '1.2.0', channel: 'stable', tag: 'v1.2.0' });
Object.assign(status.release, { version: '1.2.0', tag: 'v1.2.0' });
json(configFile, config); json(statusFile, status);
const aggregate = path.join(temporary, 'qualification-120'); mkdirSync(path.join(aggregate, 'rows'), { recursive: true });
for (let index = 0; index < 24; index++) json(path.join(aggregate, 'rows', `${index}.json`), { row: index });
const aggregateFile = path.join(aggregate, 'ait-release.clean-host-status.json');
const proof = {
  contract: 'ait.release.clean-host.aggregate/v1', status: 'qualified',
  release: { ...config.release, verification_stage: 'prepublication',
    candidate_artifact_digest: 'sha256:' + '9'.repeat(64),
    candidate_stage_receipt_sha256: status.candidate.stage_receipt_sha256, operator_status_sha256: sha(statusFile) },
  matrix: { expected_rows: 24, admitted_rows: 24, evidence_files: 24 }, failures: [],
  promotion: { allowed: true, retry_same_candidate: false, terminal_for_release: false },
};
cpSync(statusFile, path.join(aggregate, 'ait-release.prepublish-candidate.json'));
const verify = (value, pass) => {
  json(aggregateFile, value); inventory(aggregate);
  const result = spawnSync(process.execPath, [path.join(root, 'ci/release_prepublish_verify.mjs'), 'qualify',
    '--root', aggregate, '--candidate-root', candidate, '--candidate-artifact-digest', proof.release.candidate_artifact_digest,
    '--aggregate-sha256', sha(aggregateFile)], { encoding: 'utf8' });
  assert.equal(result.status === 0, pass, result.stderr);
};
verify(proof, true);
verify({ ...proof, matrix: { expected_rows: 32, admitted_rows: 32, evidence_files: 32 } }, false);
verify({ ...proof, release: { ...proof.release, channel: 'rc' } }, false);
verify({ ...proof, release: { ...proof.release, operator_status_sha256: '0'.repeat(64) } }, false);
verify({ ...proof, failures: ['failed-row'] }, false);
for (const changed of [{ channel: 'rc' }, { version: '1.2.1', tag: 'v1.2.1' }, { tag: 'v1.2.1' }]) {
  json(configFile, { ...config, release: { ...config.release, ...changed } });
  verify(proof, false);
}
SCOPED_QUALIFICATION_TEST

printf 'release prepublish contract: pass\n'

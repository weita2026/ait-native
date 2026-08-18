#!/usr/bin/env bash
set -euo pipefail

repo_root=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
promotion=${repo_root}/ci/release_protected_promotion.sh
endpoint=${repo_root}/ci/release_endpoint_publication.sh
receipts=${repo_root}/.github/workflows/ait-release-component-receipts.yml
temporary_root=$(mktemp -d "${TMPDIR:-/tmp}/ait-protected-promotion-test.XXXXXX")

cleanup() {
  case "${temporary_root}" in
    "${TMPDIR:-/tmp}"/ait-protected-promotion-test.*) rm -rf -- "${temporary_root}" ;;
    *) printf 'refusing to remove unexpected promotion test path: %s\n' \
      "${temporary_root}" >&2 ;;
  esac
}
trap cleanup EXIT HUP INT TERM

fail() {
  printf '%s\n' "$1" >&2
  exit 65
}

for required in "${promotion}" "${endpoint}"; do
  test -x "${required}" || fail "release-control script is not executable: ${required}"
  bash -n "${required}" || fail "release-control script does not parse: ${required}"
done
test -f "${receipts}" ||
  fail 'component-receipts workflow is missing'

# The frozen family dossier inventory is produced by the component-receipts
# workflow and consumed by both protected promotion and endpoint publication.
# Derive the producer inventory and require every consumer allowlist to equal
# it exactly, so a new dossier member can never reach a fail-closed consumer
# that has not been taught about it.
producer_inventory=${temporary_root}/producer-inventory
grep -o '\${dossier}/[A-Za-z0-9._-]\+' "${receipts}" |
  sed 's|^\${dossier}/||' | LC_ALL=C sort -u >"${producer_inventory}"
test -s "${producer_inventory}" ||
  fail 'component-receipts workflow publishes no family dossier member'

expected_inventory=${temporary_root}/expected-inventory
printf '%s\n' \
  ait-monorepo-source.json \
  ait-native-source-tree.tar.gz \
  ait-public-git-source.evidence.json \
  ait-release.build.json \
  ait-release.candidate.json \
  ait-release.check.json \
  ait-release.pre-tag-admission.json \
  ait-release.promotion.json \
  frozen \
  packages | LC_ALL=C sort >"${expected_inventory}"
if ! diff -u "${expected_inventory}" "${producer_inventory}"; then
  fail 'component-receipts family dossier inventory is not the expected contract'
fi

consumer_inventory() {
  local script=$1
  awk '
    /printf .%s\\n. \\$/ { count = 0; collecting = 1; next }
    collecting {
      line = $0
      sub(/^[[:space:]]+/, "", line)
      if (line ~ /LC_ALL=C sort >"\$\{expected_top\}"$/) {
        sub(/[[:space:]]*\|.*$/, "", line)
        if (line != "") { buffer[count++] = line }
        for (index_position = 0; index_position < count; index_position++) {
          print buffer[index_position]
        }
        collecting = 0
        count = 0
        next
      }
      if (line !~ /\\$/) { collecting = 0; count = 0; next }
      sub(/[[:space:]]*\\$/, "", line)
      if (line != "") { buffer[count++] = line }
    }
  ' "${script}" | LC_ALL=C sort -u
}

for consumer in "${promotion}" "${endpoint}"; do
  actual=${temporary_root}/consumer-$(basename -- "${consumer}")
  consumer_inventory "${consumer}" >"${actual}"
  test -s "${actual}" ||
    fail "consumer declares no dossier inventory allowlist: ${consumer}"
  if ! diff -u "${producer_inventory}" "${actual}"; then
    fail "dossier inventory allowlist drifted from the producer: ${consumer}"
  fi
done

# The pre-tag admission record must be semantically revalidated, not merely
# admitted into the inventory, and its digest must be bound into the
# protected-promotion evidence that endpoint publication later consumes.
for clause in \
  'ait.release.operator.pre-tag-admission/v1' \
  'ready_for_immutable_tag' \
  '.tag == {created: false, verified: false}' \
  '([.mutation[]] | all(. == false))' \
  'pre_tag_admission_sha256: $pre_tag_admission_sha256' \
  'pre_tag_admission_verified: true' \
  'pre-tag admission record does not admit this exact tagged release'; do
  grep -F -- "${clause}" "${promotion}" >/dev/null ||
    fail "protected promotion lost a pre-tag admission clause: ${clause}"
done
for clause in \
  '.dossier.pre_tag_admission_verified == true' \
  '.dossier.pre_tag_admission_sha256' \
  'family dossier pre-tag admission differs from the protected authorization'; do
  grep -F -- "${clause}" "${endpoint}" >/dev/null ||
    fail "endpoint publication lost a pre-tag admission clause: ${clause}"
done

# The dossier inventory gate runs after the protected-promotion command
# preflight, so assert that preflight independently and then satisfy it with
# appended stubs. Real tools always win the PATH lookup; the stubs only make
# the behavioural cases below reach the inventory gate on any host.
for required_command in cargo diff find git jq node rustup tar; do
  grep -F -- "for command in cargo diff find git jq node rustup tar; do" \
    "${promotion}" >/dev/null ||
    fail 'protected promotion lost its exact command preflight'
done
stub_bin=${temporary_root}/stub-bin
mkdir -p "${stub_bin}"
for required_command in cargo diff find git jq node rustup tar; do
  command -v "${required_command}" >/dev/null && continue
  printf '#!/usr/bin/env bash\nexit 70\n' >"${stub_bin}/${required_command}"
  chmod +x "${stub_bin}/${required_command}"
done
PATH=${PATH}:${stub_bin}
export PATH

# Behavioural fail-closed coverage for the dossier inventory gate.
export AIT_RELEASE_AUTHORIZATION_REF='weita2026/ait-native/.github/workflows/ait-release-protected-promotion.yml@refs/heads/main'
export AIT_RELEASE_AUTHORIZATION_RUN_ATTEMPT=1
export AIT_RELEASE_AUTHORIZATION_RUN_ID=1
export AIT_RELEASE_AUTHORIZATION_SHA=0123456789abcdef0123456789abcdef01234567
export AIT_RELEASE_CHANNEL=rc
export AIT_RELEASE_CHECKSUM_SHA256=aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa
export AIT_RELEASE_COORDINATOR_SNAPSHOT=SNP-0123456789AB
export AIT_RELEASE_DOSSIER_ARTIFACT_DIGEST=sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb
export AIT_RELEASE_DOSSIER_ARTIFACT_ID=1
export AIT_RELEASE_FROZEN_MANIFEST_SHA256=cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc
export AIT_RELEASE_GIT_COMMIT=89abcdef0123456789abcdef0123456789abcdef
export AIT_RELEASE_ID=REL-FAM-0123456789ABCDEF
export AIT_RELEASE_PROTECTED_ENVIRONMENT=rc-promotion
export AIT_RELEASE_REPOSITORY=weita2026/ait-native
export AIT_RELEASE_SOURCE_CONTROL_SHA=fedcba9876543210fedcba9876543210fedcba98
export AIT_RELEASE_SOURCE_RUN_ATTEMPT=1
export AIT_RELEASE_SOURCE_RUN_ID=1
export AIT_RELEASE_TAG=v1.0.0-rc.99

public_source_root=${temporary_root}/public-source
evidence_parent=${temporary_root}/evidence
mkdir -p "${public_source_root}" "${evidence_parent}"

build_dossier() {
  local root=$1
  shift
  rm -rf -- "${root}"
  mkdir -p "${root}"
  local member
  for member in "$@"; do
    case "${member}" in
      frozen | packages) mkdir -p "${root}/${member}" ;;
      *) printf '{}\n' >"${root}/${member}" ;;
    esac
  done
}

run_promotion() {
  local label=$1
  local dossier=$2
  rm -f -- "${evidence_parent}/${label}.json"
  set +e
  "${promotion}" "${dossier}" "${public_source_root}" \
    "${evidence_parent}/${label}.json" \
    >"${temporary_root}/${label}.stdout" 2>"${temporary_root}/${label}.stderr"
  local status=$?
  set -e
  printf '%s\n' "${status}"
}

inventory_message='family dossier top-level inventory is not exact'
contract_inventory=()
while IFS= read -r inventory_member || [[ -n ${inventory_member} ]]; do
  [[ -n ${inventory_member} ]] || continue
  contract_inventory+=("${inventory_member}")
done <"${expected_inventory}"

missing_dossier=${temporary_root}/dossier-missing
build_dossier "${missing_dossier}" \
  ait-monorepo-source.json ait-native-source-tree.tar.gz \
  ait-public-git-source.evidence.json ait-release.build.json \
  ait-release.candidate.json ait-release.check.json \
  ait-release.promotion.json frozen packages
status=$(run_promotion missing "${missing_dossier}")
test "${status}" != 0 ||
  fail 'protected promotion accepted a dossier without the pre-tag admission'
grep -F "${inventory_message}" "${temporary_root}/missing.stderr" >/dev/null ||
  fail 'missing pre-tag admission did not fail the dossier inventory gate'

extra_dossier=${temporary_root}/dossier-extra
build_dossier "${extra_dossier}" "${contract_inventory[@]}" \
  ait-release.unexpected.json
status=$(run_promotion extra "${extra_dossier}")
test "${status}" != 0 ||
  fail 'protected promotion accepted an unknown dossier member'
grep -F "${inventory_message}" "${temporary_root}/extra.stderr" >/dev/null ||
  fail 'unknown dossier member did not fail the dossier inventory gate'

exact_dossier=${temporary_root}/dossier-exact
build_dossier "${exact_dossier}" "${contract_inventory[@]}"
status=$(run_promotion exact "${exact_dossier}")
test "${status}" != 0 ||
  fail 'synthetic dossier must still fail a later protected-promotion gate'
if grep -F "${inventory_message}" "${temporary_root}/exact.stderr" >/dev/null; then
  fail 'the exact contract dossier was rejected by the inventory gate'
fi

printf 'release protected promotion contract: pass\n'

#!/usr/bin/env bash
set -euo pipefail

repo_root=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
temporary_parent=$(CDPATH='' cd -- "${TMPDIR:-/tmp}" && pwd)
temporary_root=$(mktemp -d "${temporary_parent}/ait-release-source-export.XXXXXX")
cleanup() {
  case "${temporary_root}" in
    "${temporary_parent}"/ait-release-source-export.*) rm -rf -- "${temporary_root}" ;;
    *) printf 'refusing unexpected cleanup path: %s\n' "${temporary_root}" >&2 ;;
  esac
}
trap cleanup EXIT HUP INT TERM
native=${temporary_root}/ait-native
mkdir -p "${native}/ci"
cp "${repo_root}/ait-release-family.json" "${native}/ait-release-family.json"
cp "${repo_root}/ci/release_family_bind.mjs" "${native}/ci/release_family_bind.mjs"
version=$(jq -er '.family.version' "${native}/ait-release-family.json")
cat >"${native}/ci/release_authority_preflight.sh" <<SH
#!/usr/bin/env bash
set -euo pipefail
[[ \$# -eq 2 ]]
jq -n --arg version "${version}" '
  {contract: "ait.release.canonical-authority-preflight/v2", status: "ready",
   binding: {contract: "ait.release.family-binding/v1", version: \$version, tag: ("v" + \$version),
     components: {"ait-core": "SNP-ABCDEF123456", "ait-server": "SNP-BBBBBBBBBBBB",
       "ait-runner": "SNP-CCCCCCCCCCCC", "ait-python": "SNP-DDDDDDDDDDDD", "ait-node": "SNP-EEEEEEEEEEEE"}}}
' >"\$2"
SH
cat >"${native}/ci/release_source_bundles.sh" <<'SH'
#!/usr/bin/env bash
set -euo pipefail
[[ $# -eq 2 ]]
mkdir -p "$2/ait-release-source-ait-core"
printf 'source\n' >"$2/source.txt"
printf '{"source_snapshot":"SNP-ABCDEF123456","source_manifest_hash":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","source_snapshot_created_at":"1234"}\n' \
  >"$2/ait-release-source-ait-core/source-cache.evidence.json"
SH
cat >"${native}/ci/release_monorepo_export.sh" <<'SH'
#!/usr/bin/env bash
set -euo pipefail
[[ ${AIT_RELEASE_COORDINATOR_SNAPSHOT} == SNP-ABCDEF123456 ]]
mkdir -p "$3/ci" "$3/ait-core"
cp "$1" "$3/ait-release-family.json"
printf '%s\n' "${AIT_RELEASE_VERSION_OVERLAY_FROM:-none}->${AIT_RELEASE_VERSION_OVERLAY_TO:-none}" >"$3/overlay.txt"
cat >"$3/ci/release_clean_host_test.sh" <<'TEST'
#!/usr/bin/env bash
set -euo pipefail
printf 'pass\n'
TEST
chmod +x "$3/ci/release_clean_host_test.sh"
printf '{"status":"pass"}\n' >"$4"
SH
chmod +x "${native}/ci/"*.sh

IFS=. read -r major minor patch <<<"${version}"
if (( patch > 0 )); then prior=${major}.${minor}.$((patch - 1)); else prior=${major}.$((minor - 1)).0; fi
records=${temporary_root}/records
"${repo_root}/ci/release_export_source.sh" "${native}" "${prior}" "${records}" >/dev/null
jq -e --arg version "${version}" --arg prior "${prior}" '
  .contract == "ait.release.source-export/v2" and .status == "pass" and
  .version == $version and .prior_version == $prior and
  .coordinator.snapshot_id == "SNP-ABCDEF123456" and
  .binding.components["ait-core"] == "SNP-ABCDEF123456" and
  .exports.final.version_overlay == null and
  .exports.prior.version_overlay == {from: $version, to: $prior}
' "${records}/receipt.json" >/dev/null
test ! -s "${records}/final/repeat.diff"
test ! -s "${records}/prior/repeat.diff"
test "$(cat "${records}/final/export-a/overlay.txt")" = "none->none"
test "$(cat "${records}/prior/export-a/overlay.txt")" = "${version}->${prior}"
test "$(jq -r '.family.version' "${records}/final/export-a/ait-release-family.json")" = "${version}"
test "$(jq -r '.family.version' "${records}/prior/export-a/ait-release-family.json")" = "${prior}"
test "$(jq -r '[.components[] | select(.source_repository == "ait-core") | .source_snapshot] | unique | .[0]' \
  "${records}/prior/export-a/ait-release-family.json")" = SNP-ABCDEF123456
if "${repo_root}/ci/release_export_source.sh" "${native}" "${prior}" "${records}" \
  >/dev/null 2>"${temporary_root}/repeat.stderr"; then
  printf 'source export overwrote an immutable output\n' >&2
  exit 65
fi
grep -F 'release source-export output already exists' "${temporary_root}/repeat.stderr" >/dev/null
if "${repo_root}/ci/release_export_source.sh" "${native}" "${version}" "${temporary_root}/same-version" \
  >/dev/null 2>"${temporary_root}/same.stderr"; then
  printf 'source export accepted an identical prior version\n' >&2
  exit 65
fi
grep -F 'prior version must differ' "${temporary_root}/same.stderr" >/dev/null

# Adopt a legacy completed export only after independent reproduction.
cp "${repo_root}/ci/release_export_source.sh" "${native}/ci/release_export_source.sh"
node "${repo_root}/ci/release_export_verify.mjs" "${native}" "${prior}" "${records}"
printf 'tampered\n' >>"${records}/final/export-a/overlay.txt"
if node "${repo_root}/ci/release_export_verify.mjs" "${native}" "${prior}" "${records}" >"${temporary_root}/adopt.stdout" 2>"${temporary_root}/adopt.stderr"; then
  printf 'legacy export adoption accepted drifted content\n' >&2
  exit 65
fi
grep -F 'legacy export content differs' "${temporary_root}/adopt.stderr" >/dev/null
printf 'release source-export tests passed\n'

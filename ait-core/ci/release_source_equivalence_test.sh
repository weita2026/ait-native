#!/usr/bin/env bash
set -euo pipefail

repo_root=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
temporary_parent=$(CDPATH='' cd -- "${TMPDIR:-/tmp}" && pwd)
temporary_root=$(mktemp -d "${temporary_parent}/ait-release-source-equivalence.XXXXXX")
cleanup() {
  case "${temporary_root}" in
    "${temporary_parent}"/ait-release-source-equivalence.*) rm -rf -- "${temporary_root}" ;;
    *) printf 'refusing unexpected cleanup path: %s\n' "${temporary_root}" >&2 ;;
  esac
}
trap cleanup EXIT HUP INT TERM

prior=${temporary_root}/prior
final=${temporary_root}/final
mkdir -p "${prior}/ci" "${prior}/rust" "${final}/ci" "${final}/rust"
for root in "${prior}" "${final}"; do
  cat >"${root}/ait-release-family.json" <<'JSON'
{"components":[{"id":"ait","source_snapshot":"SNP-AAAAAAAAAAAA"}]}
JSON
  printf 'same verifier\n' >"${root}/ci/release_clean_host_phase.mjs"
  printf 'same test\n' >"${root}/ci/release_clean_host_test.sh"
done
for file in \
  ait-release-family.json \
  ait-release.json \
  ci/native_bootstrap_matrix.json \
  ci/release_repository_authorities.json \
  rust/Cargo.lock \
  rust/Cargo.toml; do
  [[ ${file} == ait-release-family.json ]] || printf 'prior %s\n' "${file}" >"${prior}/${file}"
  [[ ${file} == ait-release-family.json ]] || printf 'final %s\n' "${file}" >"${final}/${file}"
done
printf '\n' >>"${final}/ait-release-family.json"

node "${repo_root}/ci/release_source_equivalence.mjs" \
  --prior "${prior}" --final "${final}" \
  --policy "${repo_root}/release/core-version-equivalence.json" \
  --output "${temporary_root}/pass.json" >/dev/null
jq -e '.status == "pass" and .nested_family_selectors_equal == true and (.changed_paths | length) == 6' \
  "${temporary_root}/pass.json" >/dev/null

printf 'regression\n' >>"${final}/ci/release_clean_host_phase.mjs"
if node "${repo_root}/ci/release_source_equivalence.mjs" \
  --prior "${prior}" --final "${final}" \
  --policy "${repo_root}/release/core-version-equivalence.json" \
  --output "${temporary_root}/rejected.json" >"${temporary_root}/rejected.stdout" 2>"${temporary_root}/rejected.stderr"; then
  printf 'source-equivalence gate accepted a verifier regression\n' >&2
  exit 65
fi
grep -F 'source-equivalence changed paths differ' "${temporary_root}/rejected.stderr" >/dev/null

printf 'release source-equivalence tests passed\n'

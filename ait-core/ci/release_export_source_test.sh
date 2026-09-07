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
core=${temporary_root}/core
mkdir -p "${core}/ci"
cat >"${core}/ait-release-family.json" <<'JSON'
{"family":{"version":"1.1.2","tag":"v1.1.2"}}
JSON
cat >"${temporary_root}/coordinator.json" <<'JSON'
{"snapshot_id":"SNP-ABCDEF123456","manifest_hash":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","created_at":"1234"}
JSON
cat >"${core}/ci/release_authority_preflight.sh" <<'SH'
#!/usr/bin/env bash
set -euo pipefail
printf '{"status":"ready"}\n' >"$2"
SH
cat >"${core}/ci/release_source_bundles.sh" <<'SH'
#!/usr/bin/env bash
set -euo pipefail
mkdir "$2"
printf 'source\n' >"$2/source.txt"
SH
cat >"${core}/ci/release_monorepo_export.sh" <<'SH'
#!/usr/bin/env bash
set -euo pipefail
mkdir -p "$3/ait-core/ci"
cp "$1" "$3/ait-release-family.json"
cat >"$3/ait-core/ci/release_clean_host_test.sh" <<'TEST'
#!/usr/bin/env bash
set -euo pipefail
printf 'pass\n'
TEST
chmod +x "$3/ait-core/ci/release_clean_host_test.sh"
printf '{"status":"pass"}\n' >"$4"
SH
chmod +x "${core}/ci/"*.sh

"${repo_root}/ci/release_export_source.sh" final "${core}" \
  "${core}/ait-release-family.json" "${temporary_root}/coordinator.json" \
  "${temporary_root}/records-final" >/dev/null
jq -e '.status == "pass" and .kind == "final" and .repeated_export_equal == true' \
  "${temporary_root}/records-final/receipt.json" >/dev/null
test ! -s "${temporary_root}/records-final/repeat.diff"
if "${repo_root}/ci/release_export_source.sh" final "${core}" \
  "${core}/ait-release-family.json" "${temporary_root}/coordinator.json" \
  "${temporary_root}/records-final" >/dev/null 2>"${temporary_root}/repeat.stderr"; then
  printf 'source export overwrote an immutable output\n' >&2
  exit 65
fi
grep -F 'release source-export output already exists' "${temporary_root}/repeat.stderr" >/dev/null

printf 'release source-export tests passed\n'

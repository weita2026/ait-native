#!/usr/bin/env bash
set -euo pipefail

repo_root=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
temporary_parent=$(CDPATH='' cd -- "${TMPDIR:-/tmp}" && pwd)
temporary_root=$(mktemp -d "${temporary_parent}/ait-release-public-source.XXXXXX")
cleanup() {
  case "${temporary_root}" in
    "${temporary_parent}"/ait-release-public-source.*) rm -rf -- "${temporary_root}" ;;
    *) printf 'refusing unexpected cleanup path: %s\n' "${temporary_root}" >&2 ;;
  esac
}
trap cleanup EXIT HUP INT TERM
remote=${temporary_root}/remote.git
public=${temporary_root}/public
prior=${temporary_root}/prior
final=${temporary_root}/final
records=${temporary_root}/records
git init --bare "${remote}" >/dev/null
git clone "${remote}" "${public}" >/dev/null 2>&1
git -C "${public}" config user.name 'AIT Release Test'
git -C "${public}" config user.email 'release-test@localhost'
printf 'base\n' >"${public}/README.md"
git -C "${public}" add README.md
git -C "${public}" commit -m base >/dev/null
git -C "${public}" branch -M main
git -C "${public}" push -u origin main >/dev/null 2>&1
mkdir -p "${prior}/ci" "${final}/ci" "${records}"
for root in "${prior}" "${final}"; do
  cat >"${root}/build-release.mjs" <<'JS'
#!/usr/bin/env node
if (!process.argv.includes("--validate-only") || !process.argv.includes("--git-commit")) process.exit(64);
JS
  cat >"${root}/ci/release_pre_rc_delta.mjs" <<'JS'
#!/usr/bin/env node
process.stdout.write('{"decision":"pass"}\n');
JS
done
cat >"${prior}/ait-release-family.json" <<'JSON'
{"family":{"version":"1.1.1","tag":"v1.1.1"}}
JSON
cat >"${final}/ait-release-family.json" <<'JSON'
{"family":{"version":"1.1.2","tag":"v1.1.2"}}
JSON
printf 'prior\n' >"${prior}/README.md"
printf 'final\n' >"${final}/README.md"

"${repo_root}/ci/release_public_source.sh" publish-qualification \
  "${public}" "${prior}" "${records}/qualification" >/dev/null
"${repo_root}/ci/release_public_source.sh" probe-qualification \
  "${public}" "${prior}" "${records}/qualification"
qualification_head=$(cat "${records}/qualification/head.txt")
test "$(git -C "${public}" rev-parse HEAD)" = "${qualification_head}"

"${repo_root}/ci/release_public_source.sh" publish-final \
  "${public}" "${final}" "${records}/qualification/head.txt" \
  "${records}/final" >/dev/null
"${repo_root}/ci/release_public_source.sh" probe-final \
  "${public}" "${final}" "${records}/qualification/head.txt" \
  "${records}/final"
final_head=$(cat "${records}/final/head.txt")
test "$(git -C "${public}" rev-parse HEAD^)" = "${qualification_head}"
cat >"${records}/candidate.json" <<JSON
{"release":{"tag":"v1.1.2","version":"1.1.2","source_commit":"${final_head}"}}
JSON
"${repo_root}/ci/release_public_source.sh" publish-tag \
  "${public}" "${records}/candidate.json" "${records}/tag" >/dev/null
"${repo_root}/ci/release_public_source.sh" probe-tag \
  "${public}" "${records}/candidate.json" "${records}/tag"
test "$(git -C "${public}" ls-remote origin 'refs/tags/v1.1.2^{}' | awk 'NR == 1 {print $1}')" = "${final_head}"

printf 'release public-source tests passed\n'
